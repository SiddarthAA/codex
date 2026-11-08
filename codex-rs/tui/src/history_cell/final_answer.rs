//! Completed turn activity retains its full details behind the existing disclosure control.

use super::*;
use std::sync::Arc;

pub(crate) fn completed_activity_range(
    cells: &[Arc<dyn HistoryCell>],
    answer_index: usize,
) -> std::ops::Range<usize> {
    let start = cells[..answer_index]
        .iter()
        .rposition(|cell| {
            cell.as_any().is::<UserHistoryCell>()
                || cell.as_any().is::<SessionInfoCell>()
                || cell.as_any().is::<FinalMessageSeparator>()
                || cell.as_any().is::<IntermediateActivityCell>()
                || cell
                    .as_any()
                    .downcast_ref::<AgentMarkdownCell>()
                    .is_some_and(AgentMarkdownCell::is_final_answer)
        })
        .map_or(0, |index| index + 1);
    start..answer_index
}

#[derive(Debug)]
pub(crate) struct IntermediateActivityCell {
    id: String,
    pub(crate) cells: Vec<Arc<dyn HistoryCell>>,
}

impl IntermediateActivityCell {
    pub(crate) fn new(turn_id: &str, cells: Vec<Arc<dyn HistoryCell>>) -> Self {
        Self {
            id: format!("completed-turn:{turn_id}"),
            cells,
        }
    }
}

impl HistoryCell for IntermediateActivityCell {
    fn warning_entries(&self) -> Vec<WarningEntry> {
        self.cells
            .iter()
            .flat_map(|cell| cell.warning_entries())
            .collect()
    }

    fn warning_keys(&self) -> Vec<WarningKey<'_>> {
        self.cells
            .iter()
            .flat_map(|cell| cell.warning_keys())
            .collect()
    }

    fn display_lines(&self, _width: u16) -> Vec<Line<'static>> {
        vec![Line::from("• Intermediate activity")]
    }

    fn activity_ids(&self) -> Vec<String> {
        vec![self.id.clone()]
    }

    fn transcript_hyperlink_lines(&self, width: u16) -> Vec<HyperlinkLine> {
        let mut lines = Vec::new();
        for cell in &self.cells {
            let details = cell.expanded_hyperlink_lines(width);
            if details.is_empty() {
                continue;
            }
            if !lines.is_empty() {
                lines.push(HyperlinkLine::new(Line::default()));
            }
            lines.extend(details);
        }
        lines
    }

    fn raw_lines(&self) -> Vec<Line<'static>> {
        self.cells
            .iter()
            .flat_map(|cell| cell.raw_lines())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn final_answer_label_reflows_without_changing_copy_source() {
        let source = "The answer is [here](https://example.test/answer).";
        let original = AgentMarkdownCell::new(source.to_owned(), Path::new("."));
        assert!(
            !original
                .display_lines(40)
                .iter()
                .any(|line| line.to_string() == "Final answer")
        );
        let final_answer = original.finalized();
        for width in [20, 40, 80] {
            let lines = final_answer.display_lines(width);
            assert_eq!(lines[0].to_string(), "─".repeat(usize::from(width)));
            assert_eq!(lines[1].to_string(), "Final answer");
            assert!(
                lines[1].spans[0]
                    .style
                    .add_modifier
                    .contains(Modifier::BOLD)
            );
            assert_eq!(final_answer.copy_source(), Some(source));
            assert_eq!(final_answer.raw_lines(), original.raw_lines());
            assert!(
                final_answer
                    .display_hyperlink_lines(width)
                    .iter()
                    .any(|line| {
                        line.hyperlinks
                            .iter()
                            .any(|link| link.destination == "https://example.test/answer")
                    })
            );
        }
    }
}
