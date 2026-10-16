//! Decoupled 60 FPS async event loop producer and stream dispatcher.
//! Decouples the UI render loop from background image decompression and MCP worker threads.

use std::sync::Arc;
use std::time::Duration;
use crossterm::event::{KeyEvent, MouseEvent};
use futures::StreamExt;
use image::DynamicImage;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::mcp::protocol::ServerInfo;

/// Unified event stream consumed by the TUI main loop.
#[derive(Debug, Clone)]
pub enum AppEvent {
    /// Keyboard input event.
    Key(KeyEvent),
    /// Mouse input event.
    Mouse(MouseEvent),
    /// Terminal window resize event.
    Resize(u16, u16),
    /// Periodic tick (~16.6ms for 60 FPS cadence).
    Tick,
    /// MCP connection state established.
    McpConnected(ServerInfo),
    /// MCP connection severed or reconnecting.
    McpDisconnected,
    /// Asynchronously decoded preview image ready for rendering.
    PreviewReady {
        photo_id: String,
        recipe_hash: u64,
        image: Arc<DynamicImage>,
    },
    /// Batch progress update from worker pool.
    BatchProgress {
        completed: usize,
        total: usize,
        current_file: String,
        percent: f32,
    },
    /// Asynchronous error notification.
    Error(String),
    /// Transient status message for the footer.
    StatusMessage(String),
    /// Explicit request to terminate the application.
    Quit,
}

/// Coordinates async input events from crossterm, tick timers, and background tasks.
pub struct EventManager {
    sender: Sender<AppEvent>,
    receiver: Receiver<AppEvent>,
    cancel_token: CancellationToken,
    _handles: Vec<JoinHandle<()>>,
}

impl EventManager {
    /// Create a new EventManager with the specified tick rate in milliseconds (typically 16ms for 60 FPS).
    pub fn new(tick_rate_ms: u64) -> Self {
        let (sender, receiver) = mpsc::channel(256);
        let cancel_token = CancellationToken::new();
        let mut handles = Vec::new();

        // 1. Spawn Crossterm EventStream task
        let tx_input = sender.clone();
        let cancel_input = cancel_token.clone();
        handles.push(tokio::spawn(async move {
            let mut reader = crossterm::event::EventStream::new();
            loop {
                tokio::select! {
                    _ = cancel_input.cancelled() => break,
                    maybe_event = reader.next() => {
                        match maybe_event {
                            Some(Ok(crossterm::event::Event::Key(k))) => {
                                if tx_input.send(AppEvent::Key(k)).await.is_err() {
                                    break;
                                }
                            }
                            Some(Ok(crossterm::event::Event::Mouse(m))) => {
                                if tx_input.send(AppEvent::Mouse(m)).await.is_err() {
                                    break;
                                }
                            }
                            Some(Ok(crossterm::event::Event::Resize(w, h))) => {
                                if tx_input.send(AppEvent::Resize(w, h)).await.is_err() {
                                    break;
                                }
                            }
                            Some(Err(_)) | None => break,
                            _ => {}
                        }
                    }
                }
            }
        }));

        // 2. Spawn 60 FPS (~16.6ms) Tick task
        let tx_tick = sender.clone();
        let cancel_tick = cancel_token.clone();
        handles.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_millis(tick_rate_ms.max(1)));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = cancel_tick.cancelled() => break,
                    _ = interval.tick() => {
                        if tx_tick.send(AppEvent::Tick).await.is_err() {
                            break;
                        }
                    }
                }
            }
        }));

        Self {
            sender,
            receiver,
            cancel_token,
            _handles: handles,
        }
    }

    /// Obtain a clone of the event sender for background workers and MCP client.
    pub fn sender(&self) -> Sender<AppEvent> {
        self.sender.clone()
    }

    /// Receive the next event asynchronously.
    pub async fn next(&mut self) -> Option<AppEvent> {
        self.receiver.recv().await
    }

    /// Stop all event producer tasks.
    pub fn shutdown(&self) {
        self.cancel_token.cancel();
    }
}

impl Drop for EventManager {
    fn drop(&mut self) {
        self.shutdown();
    }
}
