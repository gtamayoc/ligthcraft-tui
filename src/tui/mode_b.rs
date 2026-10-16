//! Mode B: Batch Queue Scaffold & Telemetry Viewer (Milestone M3 UI Contract).
//! Prepared for Milestone M4 async worker pool integration.

use std::path::PathBuf;
use std::time::{Duration, Instant};
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders, Cell, Gauge, Paragraph, Row, Table};
use ratatui::Frame;

/// Status of an individual batch processing job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchJobStatus {
    Queued,
    Processing { progress_pct: u8 },
    Done { output_path: PathBuf, bytes_written: u64, elapsed: Duration },
    Failed { error_message: String },
    Cancelled,
    Skipped { reason: String },
}

/// A single item in the batch queue table.
#[derive(Debug, Clone)]
pub struct BatchJobStub {
    pub id: u64,
    pub source_path: PathBuf,
    pub photo_id: Option<i64>,
    pub recipe_name: String,
    pub export_profile: String,
    pub status: BatchJobStatus,
    pub enqueued_at: Instant,
}

/// State container for Mode B Batch Queue.
#[derive(Debug, Clone)]
pub struct ModeBState {
    pub source_dir: PathBuf,
    pub total_photos_found: usize,
    pub strategy_label: String,
    pub queue: Vec<BatchJobStub>,
    pub selected_table_row: usize,
    pub is_running: bool,
    pub is_paused: bool,
    pub completed_count: usize,
    pub failed_count: usize,
    pub total_jobs: usize,
    pub start_time: Option<Instant>,
    pub throughput_imgs_per_sec: f32,
    pub estimated_remaining: Option<Duration>,
}

impl Default for ModeBState {
    fn default() -> Self {
        // Populate default demonstration queue items for M3 UI verification
        let sample_files = ["IMG_0588.CR3", "IMG_0589.CR3", "IMG_0590.CR3", "IMG_0969.CR3", "IMG_0981.CR3"];
        let sample_recipes = ["Natural", "Cinema", "B&W High Contrast", "Teal & Orange"];
        let mut queue = Vec::new();
        let mut id = 1;

        for file in &sample_files {
            for recipe in &sample_recipes {
                let status = if id <= 6 {
                    BatchJobStatus::Done {
                        output_path: PathBuf::from(format!("export/{}_{}.jpg", file, recipe)),
                        bytes_written: 4_200_000,
                        elapsed: Duration::from_millis(420 + id * 30),
                    }
                } else if id == 7 {
                    BatchJobStatus::Processing { progress_pct: 65 }
                } else {
                    BatchJobStatus::Queued
                };

                queue.push(BatchJobStub {
                    id,
                    source_path: PathBuf::from(format!("DRAW/{}", file)),
                    photo_id: Some(id as i64),
                    recipe_name: recipe.to_string(),
                    export_profile: "JPEG 92% (1080p)".into(),
                    status,
                    enqueued_at: Instant::now(),
                });
                id += 1;
            }
        }

        Self {
            source_dir: PathBuf::from("DRAW"),
            total_photos_found: 5,
            strategy_label: "Multi-Recipe Variations (4 variants / photo)".into(),
            total_jobs: queue.len(),
            completed_count: 6,
            failed_count: 0,
            selected_table_row: 6, // Focused on active running job
            is_running: true,
            is_paused: false,
            start_time: Some(Instant::now()),
            throughput_imgs_per_sec: 2.4,
            estimated_remaining: Some(Duration::from_secs(6)),
            queue,
        }
    }
}

impl ModeBState {
    pub fn next_row(&mut self) {
        if !self.queue.is_empty() && self.selected_table_row < self.queue.len() - 1 {
            self.selected_table_row += 1;
        }
    }

    pub fn prev_row(&mut self) {
        if self.selected_table_row > 0 {
            self.selected_table_row -= 1;
        }
    }

    pub fn toggle_pause(&mut self) {
        self.is_paused = !self.is_paused;
    }

    pub fn cancel(&mut self) {
        self.is_running = false;
        for job in &mut self.queue {
            if matches!(job.status, BatchJobStatus::Queued | BatchJobStatus::Processing { .. }) {
                job.status = BatchJobStatus::Cancelled;
            }
        }
    }

    pub fn update_from_telemetry(&mut self, telemetry: &crate::queue::worker::BatchTelemetry, _current_file: &str) {
        self.completed_count = telemetry.completed_count;
        self.failed_count = telemetry.failed_count;
        self.throughput_imgs_per_sec = telemetry.throughput_imgs_per_sec;
        self.estimated_remaining = telemetry.estimated_remaining;
    }

    pub fn set_job_status(&mut self, job_id: u64, status: BatchJobStatus) {
        if let Some(job) = self.queue.iter_mut().find(|j| j.id == job_id) {
            job.status = status;
        }
    }

    pub fn retry_failed(&mut self) -> usize {
        let mut count = 0;
        for job in &mut self.queue {
            if matches!(job.status, BatchJobStatus::Failed { .. }) {
                job.status = BatchJobStatus::Queued;
                count += 1;
            }
        }
        if count > 0 {
            self.is_running = true;
            self.is_paused = false;
        }
        count
    }

    pub fn populate_from_jobs(&mut self, jobs: &[crate::queue::job::BatchJob]) {
        self.queue = jobs
            .iter()
            .map(|j| BatchJobStub {
                id: j.id,
                source_path: j.source_path.clone(),
                photo_id: j.photo_id,
                recipe_name: j.recipe_name.clone(),
                export_profile: j.export_profile_name.clone(),
                status: match &j.status {
                    crate::queue::job::BatchJobStatus::Queued => BatchJobStatus::Queued,
                    crate::queue::job::BatchJobStatus::Developing { progress_pct } => {
                        BatchJobStatus::Processing { progress_pct: *progress_pct }
                    }
                    crate::queue::job::BatchJobStatus::Rendering { progress_pct } => {
                        BatchJobStatus::Processing { progress_pct: *progress_pct }
                    }
                    crate::queue::job::BatchJobStatus::Written { output_path, .. } => {
                        BatchJobStatus::Done {
                            output_path: output_path.clone(),
                            bytes_written: 4_200_000,
                            elapsed: Duration::from_millis(500),
                        }
                    }
                    crate::queue::job::BatchJobStatus::Done {
                        output_path,
                        bytes_written,
                        elapsed,
                    } => BatchJobStatus::Done {
                        output_path: output_path.clone(),
                        bytes_written: *bytes_written,
                        elapsed: *elapsed,
                    },
                    crate::queue::job::BatchJobStatus::Failed { error_message } => {
                        BatchJobStatus::Failed {
                            error_message: error_message.clone(),
                        }
                    }
                    crate::queue::job::BatchJobStatus::Cancelled => BatchJobStatus::Cancelled,
                    crate::queue::job::BatchJobStatus::Skipped { reason } => {
                        BatchJobStatus::Skipped {
                            reason: reason.clone(),
                        }
                    }
                },
                enqueued_at: j.enqueued_at,
            })
            .collect();
        self.total_jobs = self.queue.len();
    }
}

/// Render Mode B batch queue table, progress gauges, and controls.
pub fn render_mode_b(frame: &mut Frame, area: Rect, state: &ModeBState, is_focused: bool) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(if is_focused { BorderType::Thick } else { BorderType::Rounded })
        .border_style(if is_focused { Style::default().fg(Color::Magenta) } else { Style::default().fg(Color::DarkGray) })
        .title(" ❯ Mode B: Batch Queue Controller (M4 Async Scaffold) ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Summary banner & progress gauge
            Constraint::Length(3), // Active task status
            Constraint::Min(8),    // Queue table
            Constraint::Length(2), // Controls footer
        ])
        .split(inner);

    // 1. Summary & Global Progress
    let progress_pct = if state.total_jobs > 0 {
        ((state.completed_count as f64 / state.total_jobs as f64) * 100.0) as u16
    } else {
        0
    };

    let gauge = Gauge::default()
        .block(Block::default().title(format!(
            " Overall Batch Progress: {}/{} jobs ({:.1} img/s) ",
            state.completed_count, state.total_jobs, state.throughput_imgs_per_sec
        )))
        .gauge_style(Style::default().fg(Color::Green).bg(Color::DarkGray))
        .percent(progress_pct);

    frame.render_widget(gauge, chunks[0]);

    // 2. Active Job Status Card
    let active_desc = if state.is_paused {
        "BATCH QUEUE PAUSED. Press [Space] to resume."
    } else if let Some(active) = state.queue.iter().find(|j| matches!(j.status, BatchJobStatus::Processing { .. })) {
        &format!(
            "PROCESSING: {} -> {} ({})",
            active.source_path.file_name().unwrap_or_default().to_string_lossy(),
            active.recipe_name,
            active.export_profile
        )
    } else {
        "IDLE: All queued batch jobs processed."
    };

    let active_card = Paragraph::new(format!("  ► {}", active_desc))
        .style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        .block(Block::default().borders(Borders::BOTTOM));
    frame.render_widget(active_card, chunks[1]);

    // 3. Queue Table
    let header_cells = ["#", "Source File", "Recipe Variant", "Export Profile", "Status", "Elapsed"]
        .iter()
        .map(|h| Cell::from(*h).style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
    let header = Row::new(header_cells).height(1);

    let rows: Vec<Row> = state
        .queue
        .iter()
        .enumerate()
        .map(|(idx, job)| {
            let is_selected = idx == state.selected_table_row;
            let (status_str, status_style) = match &job.status {
                BatchJobStatus::Queued => ("Queued".to_string(), Style::default().fg(Color::DarkGray)),
                BatchJobStatus::Processing { progress_pct } => (format!("Running ({}%)", progress_pct), Style::default().fg(Color::Yellow)),
                BatchJobStatus::Done { elapsed, .. } => (format!("Done ({:.2}s)", elapsed.as_secs_f32()), Style::default().fg(Color::Green)),
                BatchJobStatus::Failed { error_message } => (format!("Failed: {}", error_message), Style::default().fg(Color::Red)),
                BatchJobStatus::Cancelled => ("Cancelled".to_string(), Style::default().fg(Color::DarkGray)),
                BatchJobStatus::Skipped { reason } => (format!("Skipped: {}", reason), Style::default().fg(Color::Blue)),
            };

            let prefix = if is_selected { "► " } else { "  " };
            let cells = vec![
                Cell::from(format!("{}{}", prefix, job.id)),
                Cell::from(job.source_path.file_name().unwrap_or_default().to_string_lossy().to_string()),
                Cell::from(job.recipe_name.clone()),
                Cell::from(job.export_profile.clone()),
                Cell::from(status_str).style(status_style),
                Cell::from(format!("{:.1}s", job.enqueued_at.elapsed().as_secs_f32())),
            ];

            let row_style = if is_selected {
                Style::default().bg(Color::Rgb(30, 30, 45)).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            Row::new(cells).style(row_style)
        })
        .collect();

    let widths = [
        Constraint::Length(6),
        Constraint::Percentage(24),
        Constraint::Percentage(24),
        Constraint::Percentage(24),
        Constraint::Percentage(14),
        Constraint::Length(8),
    ];

    let table = Table::new(rows, widths)
        .header(header)
        .block(Block::default().borders(Borders::ALL).title(" Batch Jobs "));

    frame.render_widget(table, chunks[2]);

    // 4. Action bar footer
    let footer_text = " [Space] Pause/Resume | [C] Cancel Queue | [R] Retry Failed | [▲/▼] Scroll | [Tab] Mode A ";
    let footer = Paragraph::new(footer_text)
        .alignment(Alignment::Center)
        .style(Style::default().fg(Color::Black).bg(Color::DarkGray));
    frame.render_widget(footer, chunks[3]);
}
