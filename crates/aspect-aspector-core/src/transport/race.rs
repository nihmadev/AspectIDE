use std::time::Duration;

use super::retry::backoff_delay;
use super::types::CancelRace;

/// Cancellable backoff sleep: sleeps in 250ms increments and checks `should_cancel`
/// after each tick. Returns `true` if the sleep was interrupted by cancellation.
pub async fn sleep_backoff_cancelable<C: Fn() -> bool>(attempt: u32, should_cancel: &C) -> bool {
    const TICK_MS: u64 = 250;
    let total = backoff_delay(attempt);
    let ticks = u32::try_from((total.as_millis() / u128::from(TICK_MS)).max(1)).unwrap_or(u32::MAX);
    for _ in 0..ticks {
        if should_cancel() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(TICK_MS)).await;
    }
    should_cancel()
}

/// Await `fut` while honouring a poll-based cancellation predicate and an overall
/// idle deadline. Races the future against a ticker that checks `should_cancel`
/// and a `deadline` timeout.
pub async fn race_cancel<T, Fut, C>(fut: Fut, deadline: Duration, should_cancel: &C) -> CancelRace<T>
where
    Fut: std::future::Future<Output = T>,
    C: Fn() -> bool,
{
    const CANCEL_POLL_MS: u64 = 200;
    if should_cancel() {
        return CancelRace::Cancelled;
    }
    tokio::pin!(fut);
    let started = std::time::Instant::now();
    loop {
        let remaining = match deadline.checked_sub(started.elapsed()) {
            Some(left) if !left.is_zero() => left,
            _ => return CancelRace::TimedOut,
        };
        let tick = remaining.min(Duration::from_millis(CANCEL_POLL_MS));
        tokio::select! {
            output = &mut fut => return CancelRace::Ready(output),
            () = tokio::time::sleep(tick) => {
                if should_cancel() {
                    return CancelRace::Cancelled;
                }
            }
        }
    }
}


