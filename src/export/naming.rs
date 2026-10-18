//! Template-based file naming engine and Win32 filesystem sanitization (F-EXP-03).

/// Token AST elements parsed from a naming template string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Literal(String),
    Name,
    Preset,
    Seq { padding: usize },
    Date { custom_format: Option<String> },
    Ext,
    Unknown(String),
}

/// Context passed to resolve naming tokens into concrete filename components.
#[derive(Debug, Clone)]
pub struct NamingContext<'a> {
    pub base_filename: &'a str,
    pub preset_name: &'a str,
    pub seq_number: usize,
    pub capture_date: Option<chrono::DateTime<chrono::Utc>>,
    pub extension: &'a str,
}

/// Template engine parsing and evaluating filename patterns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamingTemplate {
    pub raw: String,
    pub tokens: Vec<Token>,
}

impl Default for NamingTemplate {
    fn default() -> Self {
        Self::parse(Self::DEFAULT_TEMPLATE)
    }
}

impl NamingTemplate {
    pub const DEFAULT_TEMPLATE: &'static str = "{name}__{preset}__{seq:2}.{ext}";

    /// Parses a template string into structured tokens.
    pub fn parse(template_str: &str) -> Self {
        let trimmed = template_str.trim();
        if trimmed.is_empty() {
            return Self::parse(Self::DEFAULT_TEMPLATE);
        }

        let mut tokens = Vec::new();
        let mut chars = trimmed.chars().peekable();
        let mut literal_buf = String::new();

        while let Some(ch) = chars.next() {
            if ch == '{' {
                let mut token_buf = String::new();
                let mut closed = false;
                while let Some(&inner) = chars.peek() {
                    chars.next();
                    if inner == '}' {
                        closed = true;
                        break;
                    }
                    token_buf.push(inner);
                }

                if closed {
                    if !literal_buf.is_empty() {
                        tokens.push(Token::Literal(std::mem::take(&mut literal_buf)));
                    }
                    tokens.push(Self::parse_token(&token_buf));
                } else {
                    // Unclosed '{' treated as literal text
                    literal_buf.push('{');
                    literal_buf.push_str(&token_buf);
                }
            } else {
                literal_buf.push(ch);
            }
        }

        if !literal_buf.is_empty() {
            tokens.push(Token::Literal(literal_buf));
        }

        Self {
            raw: template_str.to_string(),
            tokens,
        }
    }

    fn parse_token(inner: &str) -> Token {
        let trimmed = inner.trim();
        if trimmed.eq_ignore_ascii_case("name") {
            Token::Name
        } else if trimmed.eq_ignore_ascii_case("preset") {
            Token::Preset
        } else if trimmed.eq_ignore_ascii_case("seq") || trimmed.eq_ignore_ascii_case("num") {
            Token::Seq { padding: 1 }
        } else if let Some(pad_str) = trimmed.strip_prefix("seq:") {
            let padding = pad_str.parse::<usize>().unwrap_or(1).clamp(1, 10);
            Token::Seq { padding }
        } else if let Some(pad_str) = trimmed.strip_prefix("num:") {
            let padding = pad_str.parse::<usize>().unwrap_or(1).clamp(1, 10);
            Token::Seq { padding }
        } else if trimmed.eq_ignore_ascii_case("date") {
            Token::Date {
                custom_format: None,
            }
        } else if let Some(fmt) = trimmed.strip_prefix("date:") {
            Token::Date {
                custom_format: Some(fmt.to_string()),
            }
        } else if trimmed.eq_ignore_ascii_case("ext") {
            Token::Ext
        } else {
            Token::Unknown(trimmed.to_string())
        }
    }

    /// Resolves template against context into a sanitized filename string.
    pub fn resolve(&self, ctx: &NamingContext<'_>) -> String {
        let mut result = String::with_capacity(64);
        let mut has_ext_token = false;

        for token in &self.tokens {
            match token {
                Token::Literal(s) => {
                    // Sanitize literal separators while preserving safe formatting
                    let mut cleaned = String::with_capacity(s.len());
                    for ch in s.chars() {
                        match ch {
                            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => {
                                cleaned.push('_')
                            }
                            c => cleaned.push(c),
                        }
                    }
                    result.push_str(&cleaned);
                }
                Token::Name => result.push_str(&sanitize_filename_component(ctx.base_filename)),
                Token::Preset => result.push_str(&sanitize_filename_component(ctx.preset_name)),
                Token::Seq { padding } => {
                    result.push_str(&format!("{:0width$}", ctx.seq_number, width = *padding));
                }
                Token::Date { custom_format } => {
                    let dt = ctx.capture_date.unwrap_or_else(chrono::Utc::now);
                    let formatted = match custom_format {
                        Some(fmt) => dt.format(fmt).to_string(),
                        None => dt.format("%Y%m%d").to_string(),
                    };
                    result.push_str(&sanitize_filename_component(&formatted));
                }
                Token::Ext => {
                    has_ext_token = true;
                    result.push_str(ctx.extension.trim_start_matches('.'));
                }
                Token::Unknown(_) => {
                    // Unknown tokens are safely omitted per specification
                }
            }
        }

        let clean_ext = ctx.extension.trim_start_matches('.');
        let trimmed_result = result.trim_end_matches('.');
        let ext_suffix = format!(".{}", clean_ext);

        if !has_ext_token
            && !trimmed_result
                .to_lowercase()
                .ends_with(&ext_suffix.to_lowercase())
        {
            format!("{}.{}", trimmed_result, clean_ext)
        } else {
            trimmed_result.to_string()
        }
    }
}

/// Sanitizes a filename component for Windows NTFS/FAT32 compatibility:
/// - Replaces illegal chars (`/ : * ? " < > | \`) with `_`
/// - Strips ASCII control chars (< 0x20)
/// - Trims trailing dots and whitespace
/// - Guards against Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1..9`, `LPT1..9`)
pub fn sanitize_filename_component(name: &str) -> String {
    let mut sanitized = String::with_capacity(name.len());
    for ch in name.chars() {
        match ch {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => sanitized.push('_'),
            c if (c as u32) < 0x20 => sanitized.push('_'),
            c => sanitized.push(c),
        }
    }
    let trimmed = sanitized.trim().trim_end_matches(['.', ' ']).to_string();
    if trimmed.is_empty() {
        return "unnamed".to_string();
    }

    let upper = trimmed.to_ascii_uppercase();
    const RESERVED: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];

    if RESERVED.contains(&upper.as_str()) {
        format!("_{}", trimmed)
    } else {
        trimmed
    }
}
