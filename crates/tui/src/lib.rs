mod app;
mod ui;

pub use app::App;

use color_eyre::Result;
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    crossterm::event::{self, Event, KeyEvent, KeyEventKind},
};
use std::{io, time::Duration};

fn pressed_key(event: &Event) -> Option<KeyEvent> {
    match event {
        Event::Key(key) if key.kind == KeyEventKind::Press => Some(*key),
        _ => None,
    }
}

/// Run the TUI application
///
/// # Errors
///
/// Returns an error if:
/// - Terminal setup fails
/// - App initialization fails
/// - Terminal operations fail during execution
/// - Cleanup operations fail
pub fn run() -> Result<()> {
    // Setup terminal
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    // Create app
    let mut app = App::new()?;

    // Setup panic hook
    let panic_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        crossterm::execute!(io::stdout(), crossterm::terminal::LeaveAlternateScreen).ok();
        crossterm::terminal::disable_raw_mode().ok();
        panic_hook(panic_info);
    }));

    // Enter alternate screen and enable raw mode
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        io::stdout(),
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )?;

    terminal.clear()?;

    // Simple event loop
    loop {
        // Draw UI
        terminal.draw(|f| ui::draw(f, &mut app))?;

        // Handle events with timeout
        if event::poll(Duration::from_millis(50))? {
            if let Ok(event) = event::read() {
                if let Some(key) = pressed_key(&event) {
                    if app.handle_key_event(key)? {
                        break;
                    }
                }
            }
        }

        // Tick for status message timeout
        app.tick();
    }

    // Cleanup
    crossterm::terminal::disable_raw_mode()?;
    crossterm::execute!(
        io::stdout(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )?;

    terminal.show_cursor()?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn preserves_consecutive_key_press_events() {
        // Terminals report unbracketed paste text as consecutive key presses.
        let pasted: String = "foobar"
            .chars()
            .filter_map(|character| {
                pressed_key(&Event::Key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE)))
            })
            .filter_map(|key| match key.code {
                KeyCode::Char(character) => Some(character),
                _ => None,
            })
            .collect();

        assert_eq!(pasted, "foobar");
    }

    #[test]
    fn ignores_non_press_key_events() {
        let release = KeyEvent::new_with_kind(KeyCode::Char('a'), KeyModifiers::NONE, KeyEventKind::Release);

        assert_eq!(pressed_key(&Event::Key(release)), None);
        assert_eq!(pressed_key(&Event::Resize(80, 24)), None);
    }
}
