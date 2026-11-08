//! Transcript consolidation for finalized streaming agent messages.
//!
//! During streaming, the chat widget emits transient `AgentMessageCell`s so it
//! can animate stable lines into scrollback while keeping the active mutable
//! tail in the bottom pane. Once the answer finishes, the app replaces that
//! trailing run with a single source-backed `AgentMarkdownCell`. This makes the
//! transcript the canonical owner of the raw markdown source used for future
//! resize re-renders. The retained view preserves its displayed revision before that
//! replacement, then repaints without replaying terminal scrollback.

use std::sync::Arc;

use color_eyre::eyre::Result;

use super::App;
use super::resize_reflow::trailing_run_start;
use crate::app_event::ConsolidationScrollbackReflow;
use crate::history_cell;
use crate::history_cell::HistoryCell;
use crate::pager_overlay::Overlay;
use crate::tui;

impl App {
    pub(super) fn finalize_answer_presentation(
        &mut self,
        tui: &mut tui::Tui,
        answer: &str,
    ) -> Result<()> {
        let Some(index) = self.transcript_cells.iter().rposition(|cell| {
            cell.as_any()
                .downcast_ref::<history_cell::AgentMarkdownCell>()
                .is_some_and(|cell| {
                    cell.copy_source().is_some_and(|source| {
                        source == answer || source.strip_suffix('\n') == Some(answer)
                    })
                })
        }) else {
            return Ok(());
        };
        let Some(cell) = self.transcript_cells[index]
            .as_any()
            .downcast_ref::<history_cell::AgentMarkdownCell>()
        else {
            return Ok(());
        };
        if cell.is_final_answer() {
            return Ok(());
        }
        let range = index..index + 1;
        let replacement: Arc<dyn HistoryCell> = Arc::new(cell.finalized());
        self.native_history
            .consolidate(&self.transcript_cells[range.clone()], &replacement);
        self.transcript_view
            .replace_group(&self.transcript_cells, range.clone(), &replacement);
        if let Some(Overlay::Transcript(overlay)) = self.overlay.as_mut() {
            overlay.regroup_cells(range.clone(), Arc::clone(&replacement));
        }
        self.transcript_cells.splice(range, [replacement]);
        if tui.is_owned_screen() {
            tui.frame_requester().schedule_frame();
        } else {
            self.finish_required_stream_reflow(tui)?;
        }
        Ok(())
    }

    pub(super) fn handle_consolidate_agent_message(
        &mut self,
        tui: &mut tui::Tui,
        source: history_cell::AgentMarkdownCell,
        scrollback_reflow: ConsolidationScrollbackReflow,
        deferred_history_cell: Option<Box<dyn HistoryCell>>,
    ) -> Result<()> {
        // Some finalize paths must preserve a last provisional stream cell long
        // enough for queue ordering, then fold it into the canonical
        // source-backed cell during consolidation.
        if let Some(cell) = deferred_history_cell {
            let cell: Arc<dyn HistoryCell> = cell.into();
            if let Some(Overlay::Transcript(t)) = &mut self.overlay {
                t.insert_cell(cell.clone());
            }
            if !tui.is_owned_screen() {
                self.native_history.defer(&cell);
            }
            self.transcript_cells.push(cell);
        }

        // Walk backward to find the contiguous run of streaming AgentMessageCells that
        // belong to the just-finalized stream.
        let end = self.transcript_cells.len();
        tracing::debug!(
            "ConsolidateAgentMessage: transcript_cells.len()={end}, source_len={}",
            source.copy_source().map_or(/*default*/ 0, str::len)
        );
        let start = trailing_run_start::<history_cell::AgentMessageCell>(&self.transcript_cells);
        if start < end {
            tracing::debug!(
                "ConsolidateAgentMessage: replacing cells [{start}..{end}] with AgentMarkdownCell"
            );
            let consolidated: Arc<dyn HistoryCell> = Arc::new(source);
            self.native_history
                .consolidate(&self.transcript_cells[start..end], &consolidated);
            if tui.is_owned_screen() {
                self.transcript_view.replace_group(
                    &self.transcript_cells,
                    start..end,
                    &consolidated,
                );
            } else {
                self.transcript_view.replace_range(
                    &self.transcript_cells,
                    start..end,
                    &consolidated,
                );
            }
            self.transcript_cells
                .splice(start..end, std::iter::once(consolidated.clone()));

            if let Some(Overlay::Transcript(t)) = &mut self.overlay {
                t.regroup_cells(start..end, consolidated.clone());
                tui.frame_requester().schedule_frame();
            }

            self.finish_agent_message_consolidation(tui, scrollback_reflow)?;
        } else {
            tracing::debug!(
                "ConsolidateAgentMessage: no cells to consolidate(start={start}, end={end})",
            );
            self.maybe_finish_stream_reflow(tui)?;
        }

        Ok(())
    }

    fn finish_agent_message_consolidation(
        &mut self,
        tui: &mut tui::Tui,
        scrollback_reflow: ConsolidationScrollbackReflow,
    ) -> Result<()> {
        if tui.is_owned_screen() {
            self.transcript_reflow.clear_pending_reflow();
            self.transcript_reflow.clear_stream_flags();
            tui.frame_requester().schedule_frame();
            return Ok(());
        }
        match scrollback_reflow {
            ConsolidationScrollbackReflow::IfResizeReflowRan => {
                self.maybe_finish_stream_reflow(tui)?;
            }
            ConsolidationScrollbackReflow::Required => {
                self.finish_required_stream_reflow(tui)?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod final_answer_tests {
    use super::*;
    use std::path::Path;

    #[tokio::test]
    async fn completion_keeps_activity_visible_and_separates_the_answer() -> Result<()> {
        let mut app = crate::app::test_support::make_test_app().await;
        let mut tui = crate::tui::test_support::make_test_tui()?;
        tui.set_owned_screen(true)?;
        let previous: Arc<dyn HistoryCell> = Arc::new(
            history_cell::AgentMarkdownCell::new("Previous answer".into(), Path::new("."))
                .finalized(),
        );
        let prompt: Arc<dyn HistoryCell> = Arc::new(history_cell::new_user_prompt(
            "Current prompt".into(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        ));
        let progress: Arc<dyn HistoryCell> = Arc::new(history_cell::AgentMarkdownCell::new(
            "Working on it".into(),
            Path::new("."),
        ));
        let answer: Arc<dyn HistoryCell> = Arc::new(history_cell::AgentMarkdownCell::new(
            "Done\n".into(),
            Path::new("."),
        ));
        let timestamp: Arc<dyn HistoryCell> =
            Arc::new(history_cell::FinalMessageSeparator::new(Some(5), None));
        app.transcript_cells = vec![
            previous.clone(),
            prompt.clone(),
            progress.clone(),
            answer,
            timestamp.clone(),
        ];

        app.finalize_answer_presentation(&mut tui, "missing answer")?;
        assert_eq!(app.transcript_cells.len(), 5);
        app.finalize_answer_presentation(&mut tui, "Done")?;
        assert_eq!(app.transcript_cells.len(), 5);
        assert!(Arc::ptr_eq(&app.transcript_cells[0], &previous));
        assert!(Arc::ptr_eq(&app.transcript_cells[1], &prompt));
        assert!(Arc::ptr_eq(&app.transcript_cells[2], &progress));
        assert!(
            app.transcript_cells[2]
                .display_lines(40)
                .iter()
                .any(|line| { line.to_string().contains("Working on it") })
        );
        let answer = app.transcript_cells[3]
            .as_any()
            .downcast_ref::<history_cell::AgentMarkdownCell>()
            .unwrap();
        assert!(answer.is_final_answer());
        assert_eq!(answer.copy_source(), Some("Done\n"));
        assert!(Arc::ptr_eq(&app.transcript_cells[4], &timestamp));
        let answer = app.transcript_cells[3].clone();
        app.finalize_answer_presentation(&mut tui, "Done")?;
        assert!(Arc::ptr_eq(&app.transcript_cells[3], &answer));
        Ok(())
    }
}
