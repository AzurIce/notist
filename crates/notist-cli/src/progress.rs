use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use notist::progress::{Event, Status};
use std::{cell::RefCell, io::IsTerminal, time::Duration};

/// Progress goes exclusively to stderr so machine-readable stdout stays intact.
pub(crate) fn observe<T>(verbose: bool, f: impl FnOnce() -> T) -> T {
    if !verbose {
        return f();
    }
    let interactive = std::io::stderr().is_terminal()
        && std::env::var_os("CI").is_none()
        && std::env::var("TERM").as_deref() != Ok("dumb");
    let display = MultiProgress::with_draw_target(if interactive {
        ProgressDrawTarget::stderr()
    } else {
        ProgressDrawTarget::hidden()
    });
    let active = RefCell::new(Vec::<(ProgressBar, usize)>::new());
    let tree = RefCell::new(Vec::<String>::new());
    notist::progress::observe(
        move |event: &Event<'_>| {
            let prefix = if event.depth == 0 {
                String::new()
            } else {
                format!("{}├─ ", "│  ".repeat(event.depth - 1))
            };
            match event.status {
                Status::Started => {
                    if interactive {
                        let bar = display.add(ProgressBar::new_spinner());
                        bar.set_style(
                            ProgressStyle::with_template(
                                "{prefix}{spinner} {msg} [{elapsed_precise}]",
                            )
                            .unwrap(),
                        );
                        bar.set_prefix(prefix.clone());
                        bar.set_message(event.label.to_owned());
                        bar.enable_steady_tick(Duration::from_millis(80));
                        let index = tree.borrow().len();
                        tree.borrow_mut().push(format!("{prefix}… {}", event.label));
                        active.borrow_mut().push((bar, index));
                    } else {
                        eprintln!("{prefix}… {}", event.label);
                    }
                }
                status => {
                    let marker = if status == Status::Finished {
                        "✓"
                    } else {
                        "✗"
                    };
                    if interactive {
                        if let Some((bar, index)) = active.borrow_mut().pop() {
                            tree.borrow_mut()[index] = format!(
                                "{prefix}{marker} {} [{:.3}s]",
                                event.label,
                                event.elapsed.as_secs_f64()
                            );
                            // Only active ancestors stay in the live display.
                            // Keeping every finished bar would make redraw work
                            // and ticker state grow with the number of pages.
                            bar.finish_and_clear();
                            display.remove(&bar);
                        }
                        if event.depth == 0 {
                            // Commit the complete tree to scrollback, including
                            // rows hidden by terminal height, and release bars
                            // before the next serve rebuild.
                            let lines = tree.take().join("\n");
                            let _ = display.clear();
                            let _ = display.println(lines);
                        }
                    } else {
                        eprintln!(
                            "{prefix}{marker} {} [{:.3}s]",
                            event.label,
                            event.elapsed.as_secs_f64()
                        );
                    }
                }
            }
        },
        f,
    )
}
