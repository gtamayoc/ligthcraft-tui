//! Windows native Toast notification dispatch via PowerShell WinRT XML DOM.
//!
//! Provides non-blocking, windowless (`CREATE_NO_WINDOW`) toast notifications
//! to inform the user when an automated SD ingest process has finished.

use std::sync::{Arc, Mutex};
use super::IngestError;

/// Escape XML special characters to prevent XML injection or syntax errors in WinRT DOM.
pub fn escape_xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Encode a PowerShell script into a UTF-16LE Base64 string for `-EncodedCommand`.
pub fn encode_powershell_script(script: &str) -> String {
    use base64::Engine;
    let utf16_bytes: Vec<u8> = script
        .encode_utf16()
        .flat_map(|u| u.to_le_bytes())
        .collect();
    base64::engine::general_purpose::STANDARD.encode(&utf16_bytes)
}

/// Trait abstracting OS toast notification dispatch.
pub trait ToastNotifier: Send + Sync {
    /// Send a native toast notification with title and message body.
    fn send_toast(&self, title: &str, message: &str) -> Result<(), IngestError>;
}

/// Windows native Toast Notifier using PowerShell WinRT XML DOM and `CREATE_NO_WINDOW`.
#[derive(Debug, Clone)]
pub struct WindowsToastNotifier {
    pub app_id: String,
}

impl WindowsToastNotifier {
    /// Standard Windows Start Menu AppUserModelId for Windows PowerShell.
    pub const DEFAULT_APP_ID: &'static str =
        "{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\WindowsPowerShell\\v1.0\\powershell.exe";

    pub fn new() -> Self {
        Self {
            app_id: Self::DEFAULT_APP_ID.to_string(),
        }
    }

    pub fn with_app_id(app_id: impl Into<String>) -> Self {
        Self {
            app_id: app_id.into(),
        }
    }
}

impl Default for WindowsToastNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl ToastNotifier for WindowsToastNotifier {
    fn send_toast(&self, title: &str, message: &str) -> Result<(), IngestError> {
        #[cfg(windows)]
        {
            let escaped_title = escape_xml(title);
            let escaped_message = escape_xml(message);

            let script = format!(
                r#"[void][Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime]
[void][Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime]
$template = @"
<toast>
    <visual>
        <binding template="ToastGeneric">
            <text>{}</text>
            <text>{}</text>
        </binding>
    </visual>
</toast>
"@
$xml = New-Object Windows.Data.Xml.Dom.XmlDocument
$xml.LoadXml($template)
$toast = [Windows.UI.Notifications.ToastNotification]::new($xml)
$notifier = [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('{}')
$notifier.Show($toast)
"#,
                escaped_title, escaped_message, self.app_id
            );

            let encoded_command = encode_powershell_script(&script);

            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x08000000;

            let mut cmd = std::process::Command::new("powershell");
            cmd.args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-EncodedCommand",
                &encoded_command,
            ]);
            cmd.creation_flags(CREATE_NO_WINDOW);

            let mut child = cmd.spawn().map_err(|e| {
                tracing::warn!("Failed to spawn PowerShell toast process: {}", e);
                IngestError::ToastError(format!("Failed to spawn PowerShell toast process: {}", e))
            })?;

            // Detach wait in background thread to avoid zombie processes
            std::thread::spawn(move || {
                let _ = child.wait();
            });

            Ok(())
        }

        #[cfg(not(windows))]
        {
            let _ = (title, message);
            tracing::info!("Toast notification dispatched (non-Windows mock): [{}] {}", title, message);
            Ok(())
        }
    }
}

/// Mock Toast Notifier for deterministic testing without launching desktop toasts.
#[derive(Debug, Clone)]
pub struct MockToastNotifier {
    pub sent_toasts: Arc<Mutex<Vec<(String, String)>>>,
    pub records: Arc<Mutex<Vec<(String, String)>>>,
    pub should_fail: Arc<std::sync::atomic::AtomicBool>,
}

impl Default for MockToastNotifier {
    fn default() -> Self {
        Self::new()
    }
}

impl MockToastNotifier {
    pub fn new() -> Self {
        let store = Arc::new(Mutex::new(Vec::new()));
        Self {
            sent_toasts: Arc::clone(&store),
            records: store,
            should_fail: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn get_sent_toasts(&self) -> Vec<(String, String)> {
        self.sent_toasts.lock().unwrap().clone()
    }

    pub fn sent_toasts(&self) -> Vec<(String, String)> {
        self.get_sent_toasts()
    }

    pub fn last_notification(&self) -> Option<(String, String)> {
        self.sent_toasts.lock().unwrap().last().cloned()
    }

    pub fn count(&self) -> usize {
        self.sent_toasts.lock().unwrap().len()
    }

    pub fn set_should_fail(&self, fail: bool) {
        self.should_fail.store(fail, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn clear(&self) {
        self.sent_toasts.lock().unwrap().clear();
    }
}

impl ToastNotifier for MockToastNotifier {
    fn send_toast(&self, title: &str, message: &str) -> Result<(), IngestError> {
        if self.should_fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(IngestError::ToastError("Simulated toast failure".to_string()));
        }
        self.sent_toasts
            .lock()
            .unwrap()
            .push((title.to_string(), message.to_string()));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_xml_escaping_all_special_chars() {
        let raw = r#"<tag attr="val" & 'quote'>Canon & Co</tag>"#;
        let escaped = escape_xml(raw);
        assert_eq!(
            escaped,
            "&lt;tag attr=&quot;val&quot; &amp; &apos;quote&apos;&gt;Canon &amp; Co&lt;/tag&gt;"
        );
    }

    #[test]
    fn test_encode_powershell_script_roundtrip() {
        use base64::Engine;
        let script = "Write-Host 'Hello, World!'";
        let encoded = encode_powershell_script(script);
        let decoded_bytes = base64::engine::general_purpose::STANDARD
            .decode(&encoded)
            .expect("base64 decode");
        let utf16_units: Vec<u16> = decoded_bytes
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        let decoded_str = String::from_utf16(&utf16_units).expect("utf16 decode");
        assert_eq!(decoded_str, script);
    }

    #[test]
    fn test_mock_toast_notifier_lifecycle() {
        let notifier = MockToastNotifier::new();
        assert_eq!(notifier.count(), 0);
        assert_eq!(notifier.last_notification(), None);

        notifier.send_toast("Title 1", "Body 1").expect("send 1");
        assert_eq!(notifier.count(), 1);
        assert_eq!(
            notifier.last_notification(),
            Some(("Title 1".to_string(), "Body 1".to_string()))
        );

        notifier.send_toast("Title 2", "Body 2").expect("send 2");
        assert_eq!(notifier.count(), 2);
        assert_eq!(
            notifier.last_notification(),
            Some(("Title 2".to_string(), "Body 2".to_string()))
        );

        let sent = notifier.get_sent_toasts();
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[0].0, "Title 1");
        assert_eq!(sent[1].0, "Title 2");

        // Verify records alias shares same data
        assert_eq!(notifier.records.lock().unwrap().len(), 2);

        notifier.clear();
        assert_eq!(notifier.count(), 0);
        assert_eq!(notifier.last_notification(), None);
    }

    #[test]
    fn test_mock_toast_notifier_should_fail() {
        let notifier = MockToastNotifier::new();
        notifier.set_should_fail(true);
        let res = notifier.send_toast("Title", "Body");
        assert!(res.is_err());
        match res.unwrap_err() {
            IngestError::ToastError(msg) => {
                assert!(msg.contains("Simulated toast failure"));
            }
            other => panic!("Unexpected error type: {:?}", other),
        }
    }

    #[test]
    fn test_mock_toast_non_blocking_speed() {
        let notifier = MockToastNotifier::new();
        let start = Instant::now();
        for i in 0..100 {
            notifier.send_toast(&format!("Title {}", i), "Message").expect("send");
        }
        let elapsed = start.elapsed();
        // 100 dispatches should be sub-millisecond, definitely under 50ms
        assert!(elapsed < std::time::Duration::from_millis(50));
    }

    #[test]
    fn test_toast_notifier_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<WindowsToastNotifier>();
        assert_send_sync::<MockToastNotifier>();
        assert_send_sync::<Box<dyn ToastNotifier>>();
        assert_send_sync::<Arc<dyn ToastNotifier>>();
    }

    #[test]
    fn test_windows_toast_notifier_creation() {
        let default_notifier = WindowsToastNotifier::new();
        assert_eq!(default_notifier.app_id, WindowsToastNotifier::DEFAULT_APP_ID);

        let custom_notifier = WindowsToastNotifier::with_app_id("CustomAppId");
        assert_eq!(custom_notifier.app_id, "CustomAppId");
    }
}
