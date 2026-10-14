//! `Ui::with_progress`: runs work on a scoped thread while the calling
//! thread draws the live display.

use crate::progress::ProgressSink;
use crate::ui::Ui;

impl Ui {
    /// Runs `work` on a scoped thread with a [`ProgressSink`], while this
    /// thread folds its events into a `ProgressView` and (in `Tty` mode)
    /// redraws it in place on stderr every 80 ms, until the channel
    /// disconnects. Then clears the area, prints the summary line (`Tty`
    /// only), and flushes deferred warnings (both modes). `title` is shown
    /// until the work sends a `Header`.
    ///
    /// # Panics
    ///
    /// If `work` panics, the display is cleared, the cursor restored, and
    /// warnings flushed, then the panic is resumed.
    pub fn with_progress<R: Send>(
        &mut self,
        title: &str,
        work: impl FnOnce(&ProgressSink) -> R + Send,
    ) -> R {
        let _ = (self, title, work);
        unimplemented!("W1.7")
    }
}
