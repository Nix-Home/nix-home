use std::io;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen};

use super::Table;

/// Returns a slice of `line` covering up to `width` characters starting at
/// character position `start`. Positions are counted in characters (not
/// bytes), so multi-byte host names cannot panic or corrupt output.
pub(crate) fn viewport_line(line: &str, start: usize, width: usize) -> String {
    line.char_indices()
        .skip(start)
        .take(width)
        .map(|(_, c)| c)
        .collect()
}

/// Enters raw mode and the alternate screen; restores the terminal on drop,
/// whatever the reason for leaving (normal return or panic).
struct RawTerminal {
    active: bool,
}

impl RawTerminal {
    fn enter() -> io::Result<Self> {
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self { active: true })
    }
}

impl Drop for RawTerminal {
    fn drop(&mut self) {
        if self.active {
            let _ = execute!(io::stdout(), Show, LeaveAlternateScreen);
            let _ = terminal::disable_raw_mode();
            self.active = false;
        }
    }
}

/// How many characters a single left/right key press scrolls.
const HORIZONTAL_STEP: usize = 10;

/// Renders the table in a scrollable viewport inside the alternate screen.
/// Left/right (h/l) scroll horizontally, up/down (j/k) vertically, with
/// PageUp/PageDown/Home/End jumps. `q`, `Esc` or Ctrl-C leaves.
pub fn launch(table: &Table) -> io::Result<()> {
    let _terminal = RawTerminal::enter()?;
    run(table)
}

fn run(table: &Table) -> io::Result<()> {
    let lines = table_lines(table);
    let total_width = lines
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);

    let mut stdout = io::stdout();
    let mut col: usize = 0;
    let mut row: usize = 0;

    loop {
        let (term_cols, term_rows) = terminal::size()?;
        let view_cols = term_cols as usize;
        // Leave the bottom line for the hint.
        let visible_rows = term_rows.saturating_sub(2) as usize;

        let max_col = total_width.saturating_sub(view_cols);
        col = col.min(max_col);
        let max_row = lines.len().saturating_sub(visible_rows.max(1));
        row = row.min(max_row);

        if visible_rows > 0 {
            execute!(stdout, Clear(ClearType::All))?;
            for (i, offset) in (row..row + visible_rows).enumerate() {
                if offset >= lines.len() {
                    break;
                }
                let text = viewport_line(&lines[offset], col, view_cols);
                execute!(stdout, MoveTo(0, i as u16), Print(text))?;
            }
            let hint = format!("  ← → ↑ ↓ scroll · q quit   (col {col}, row {row})");
            execute!(stdout, MoveTo(0, term_rows - 1), Print(hint))?;
        }

        let event = event::read()?;
        match event {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                KeyCode::Left | KeyCode::Char('h') => col = col.saturating_sub(HORIZONTAL_STEP),
                KeyCode::Right | KeyCode::Char('l') => {
                    col = col.saturating_add(HORIZONTAL_STEP).min(max_col)
                }
                KeyCode::Up | KeyCode::Char('k') => row = row.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => row = row.saturating_add(1).min(max_row),
                KeyCode::PageUp => {
                    let page = visible_rows.saturating_sub(1).max(1);
                    row = row.saturating_sub(page);
                }
                KeyCode::PageDown => {
                    let page = visible_rows.saturating_sub(1).max(1);
                    row = row.saturating_add(page).min(max_row);
                }
                KeyCode::Home => row = 0,
                KeyCode::End => row = max_row,
                _ => {}
            },
            Event::Resize(_, _) => continue,
            _ => {}
        }
    }
}

fn table_lines(table: &Table) -> Vec<String> {
    let mut lines = vec![Table::render_row(&table.header, &table.widths)];
    for row in &table.rows {
        lines.push(Table::render_row(row, &table.widths));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_line_returns_full_width_when_content_fits() {
        assert_eq!(viewport_line("hello", 0, 10), "hello");
    }

    #[test]
    fn viewport_line_skips_to_start_offset() {
        assert_eq!(viewport_line("hello world", 6, 10), "world");
    }

    #[test]
    fn viewport_line_clamps_at_end_of_line() {
        assert_eq!(viewport_line("hello", 3, 10), "lo");
    }

    #[test]
    fn viewport_line_limits_to_requested_width() {
        assert_eq!(viewport_line("hello world", 0, 5), "hello");
    }

    #[test]
    fn viewport_line_handles_zero_width_and_past_end_start() {
        assert_eq!(viewport_line("hello", 0, 0), "");
        assert_eq!(viewport_line("hello", 100, 10), "");
    }

    #[test]
    fn viewport_line_is_utf8_safe() {
        let line = "höl-üñiçode";
        assert_eq!(viewport_line(line, 2, 3), "l-ü");
        assert_eq!(viewport_line(line, 0, 100).chars().count(), line.chars().count());
    }
}
