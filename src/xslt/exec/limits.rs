//! Limits on a transform: the template recursion limit, and the work budget
//! and timeout the library can set (`Limits`; the command line sets none, as
//! xsltproc has none).

use super::*;

/// libxslt's default `--maxdepth`: nested template calls.
pub const MAX_DEPTH: usize = 3_000;

/// Limits on a transform's work and on what it may write. The default is
/// no work limit, as in xsltproc, and no file writes, so a library caller
/// that doesn't ask for writes never gets any.
#[derive(Debug, Clone, Default)]
pub struct Limits {
    /// Units of work: one per instruction, per node a template is applied
    /// to, per XPath evaluation, per node an XPath step visits, per node
    /// copied or indexed for a key, and per 64 bytes `concat()` builds.
    pub max_work: Option<u64>,
    /// Wall-clock time for the whole transform.
    pub timeout: Option<Duration>,
    /// Which files `exsl:document` may write.
    pub writes: Writes,
}

/// A transform's remaining work and its deadline, shared with XPath
/// evaluation through `Functions::tick`.
pub(super) struct Budget {
    /// Whether any limit is set; without one, a tick does nothing.
    active: bool,
    left: Cell<u64>,
    limits: Limits,
    deadline: Option<Instant>,
    /// Ticks until the clock is read again.
    until_clock: Cell<u32>,
    /// Set once a limit is exceeded: the message.
    pub(super) exceeded: RefCell<Option<String>>,
}

impl Budget {
    pub(super) fn new(limits: &Limits) -> Self {
        let limits = Limits {
            writes: Writes::default(),
            ..limits.clone()
        };
        Budget {
            active: limits.max_work.is_some() || limits.timeout.is_some(),
            left: Cell::new(limits.max_work.unwrap_or(u64::MAX)),
            deadline: limits.timeout.map(|t| Instant::now() + t),
            limits,
            until_clock: Cell::new(4096),
            exceeded: RefCell::new(None),
        }
    }

    /// Counts `cost` work units. Called for every instruction and XPath
    /// step, so the common case, no limits, is a single test.
    #[inline]
    pub(super) fn tick(&self, cost: u64) -> Result<(), String> {
        if !self.active {
            return Ok(());
        }
        self.tick_limited(cost)
    }

    #[cold]
    #[inline(never)]
    fn tick_limited(&self, cost: u64) -> Result<(), String> {
        if let Some(e) = &*self.exceeded.borrow() {
            return Err(e.clone());
        }
        if let Some(max) = self.limits.max_work {
            let left = self.left.get().saturating_sub(cost);
            self.left.set(left);
            if left == 0 {
                return Err(self.exceed(format!(
                    "Transform stopped: it exceeded the work limit of {max} units"
                )));
            }
        }
        if let Some(deadline) = self.deadline {
            let n = self.until_clock.get().saturating_sub(1);
            self.until_clock.set(if n == 0 { 4096 } else { n });
            if n == 0 && Instant::now() > deadline {
                let t = self.limits.timeout.unwrap_or_default();
                return Err(self.exceed(format!(
                    "Transform stopped: it exceeded the time limit of {t:?}"
                )));
            }
        }
        Ok(())
    }

    fn exceed(&self, message: String) -> String {
        *self.exceeded.borrow_mut() = Some(message.clone());
        message
    }
}
