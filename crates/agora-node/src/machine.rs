//! ADR-0136: the machine's CPU, memory and disks in the node's `NodeResources`. Read every [`CHECK`] off the hot paths; said only
//! when a figure moved past its step and at most once per [`MIN_GAP`], so a quiet machine adds almost nothing to the event log.

use std::time::{Duration, Instant};

use agora_protocol::v1;

/// How often the machine is read.
pub const CHECK: Duration = Duration::from_secs(30);
/// The least time between two reports of the machine's figures.
pub const MIN_GAP: Duration = Duration::from_secs(60);
const GB: u64 = 1 << 30;

/// The machine's figures as `NodeResources` carries them (zero: not reported).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Figures {
    pub cpu_count: u32,
    pub cpu_busy_permille: u32,
    pub memory_total_bytes: u64,
    pub memory_available_bytes: u64,
    pub disks: Vec<v1::DiskUsage>,
    pub workspaces_bytes: u64,
}

impl Figures {
    /// Whether `self` differs from `before` enough to say: CPU by 5 points, memory by 2 % of its total, a disk by 1 % of its size or
    /// 1 GB, or a disk appearing or going (or its marks changing).
    pub fn moved_from(&self, before: &Figures) -> bool {
        let cpu = self.cpu_busy_permille.abs_diff(before.cpu_busy_permille) >= 50
            || self.cpu_count != before.cpu_count;
        let mem_step = (self.memory_total_bytes / 50).max(1);
        let mem = self.memory_total_bytes != before.memory_total_bytes
            || self
                .memory_available_bytes
                .abs_diff(before.memory_available_bytes)
                >= mem_step;
        let disks = self.disks.len() != before.disks.len()
            || self.disks.iter().zip(&before.disks).any(|(a, b)| {
                a.mount != b.mount
                    || a.holds_data != b.holds_data
                    || a.holds_workspaces != b.holds_workspaces
                    || a.total_bytes != b.total_bytes
                    || a.free_bytes.abs_diff(b.free_bytes) >= (a.total_bytes / 100).clamp(1, GB)
            });
        let ws = self.workspaces_bytes.abs_diff(before.workspaces_bytes) >= GB;
        cpu || mem || disks || ws
    }
}

/// Reads the machine and decides when its figures are worth saying.
#[derive(Default)]
pub struct MachineGate {
    counters: Option<(u64, u64)>,
    checked: Option<Instant>,
    said: Option<(Figures, Instant)>,
    /// The latest figures read, said or not, so a report made for another reason (the running agents changed) carries them too.
    pub latest: Figures,
}

impl MachineGate {
    /// Whether it is time to read the machine again.
    pub fn due(&self, now: Instant) -> bool {
        self.checked
            .is_none_or(|t| now.saturating_duration_since(t) >= CHECK)
    }

    /// Takes one reading: CPU counters (the busy share is worked out against the previous reading; the first gives none), memory, disks
    /// and the workspaces' size. A part the platform could not read is left at zero.
    pub fn read(
        &mut self,
        now: Instant,
        stats: Option<agora_platform::MachineStats>,
        disks: &[agora_platform::DiskSpace],
        data_mark: usize,
        workspaces_mark: usize,
        workspaces_bytes: u64,
    ) {
        self.checked = Some(now);
        let mut f = Figures {
            workspaces_bytes,
            ..Figures::default()
        };
        if let Some(s) = stats {
            f.cpu_count = s.cpu_count;
            f.memory_total_bytes = s.memory_total_bytes;
            f.memory_available_bytes = s.memory_available_bytes.min(s.memory_total_bytes);
            if let Some((busy, total)) = self.counters {
                let (db, dt) = (
                    s.cpu_busy_ticks.saturating_sub(busy),
                    s.cpu_total_ticks.saturating_sub(total),
                );
                if let Some(share) = db.min(dt).saturating_mul(1000).checked_div(dt) {
                    f.cpu_busy_permille = u32::try_from(share).unwrap_or(1000);
                }
            }
            self.counters = Some((s.cpu_busy_ticks, s.cpu_total_ticks));
        }
        f.disks = disks
            .iter()
            .map(|d| v1::DiskUsage {
                mount: d.mount.clone(),
                total_bytes: d.total_bytes,
                free_bytes: d.free_bytes.min(d.total_bytes),
                holds_data: d.holds.contains(&data_mark),
                holds_workspaces: d.holds.contains(&workspaces_mark),
            })
            .collect();
        self.latest = f;
    }

    /// Whether the latest figures should be said now: the first time, or when they moved past a step and the last report is at least
    /// [`MIN_GAP`] old. Remembers them as said when it answers yes.
    pub fn should_say(&mut self, now: Instant) -> bool {
        let yes = match &self.said {
            None => self.latest != Figures::default(),
            Some((before, at)) => {
                now.saturating_duration_since(*at) >= MIN_GAP && self.latest.moved_from(before)
            }
        };
        if yes {
            self.said = Some((self.latest.clone(), now));
        }
        yes
    }

    /// The latest figures were said with another report.
    pub fn said_with_other(&mut self, now: Instant) {
        if self.latest != Figures::default() {
            self.said = Some((self.latest.clone(), now));
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn stats(busy: u64, total: u64, avail: u64) -> agora_platform::MachineStats {
        agora_platform::MachineStats {
            cpu_count: 8,
            cpu_busy_ticks: busy,
            cpu_total_ticks: total,
            memory_total_bytes: 100 * GB,
            memory_available_bytes: avail,
        }
    }

    fn disk(free: u64) -> agora_platform::DiskSpace {
        agora_platform::DiskSpace {
            mount: "/".into(),
            total_bytes: 500 * GB,
            free_bytes: free,
            holds: vec![0, 1],
        }
    }

    #[test]
    fn the_busy_share_comes_from_two_readings_and_marks_follow_the_disk() {
        let mut g = MachineGate::default();
        let t0 = Instant::now();
        assert!(g.due(t0));
        g.read(
            t0,
            Some(stats(100, 1000, 60 * GB)),
            &[disk(40 * GB)],
            0,
            1,
            3 * GB,
        );
        assert_eq!(g.latest.cpu_busy_permille, 0, "no share from one reading");
        assert!(!g.due(t0 + Duration::from_secs(10)));
        g.read(
            t0 + CHECK,
            Some(stats(400, 2000, 60 * GB)),
            &[disk(40 * GB)],
            0,
            1,
            3 * GB,
        );
        assert_eq!(g.latest.cpu_busy_permille, 300);
        assert!(g.latest.disks[0].holds_data && g.latest.disks[0].holds_workspaces);
        g.read(t0 + CHECK * 2, None, &[], 0, 1, 0);
        assert_eq!(
            g.latest,
            Figures::default(),
            "nothing read, nothing reported"
        );
    }

    #[test]
    fn figures_are_said_first_then_only_when_they_move_and_not_too_often() {
        let mut g = MachineGate::default();
        let t0 = Instant::now();
        assert!(!g.should_say(t0), "nothing read yet");
        g.read(t0, Some(stats(0, 1000, 60 * GB)), &[disk(40 * GB)], 0, 1, 0);
        assert!(g.should_say(t0));
        // Small moves stay quiet.
        g.read(
            t0 + MIN_GAP,
            Some(stats(10, 2000, 59 * GB)),
            &[disk(40 * GB - (GB / 2))],
            0,
            1,
            0,
        );
        assert!(!g.should_say(t0 + MIN_GAP));
        // A disk losing 1 GB is said, but not within a minute of the last report.
        g.read(
            t0 + Duration::from_secs(30),
            Some(stats(20, 3000, 59 * GB)),
            &[disk(38 * GB)],
            0,
            1,
            0,
        );
        assert!(!g.should_say(t0 + Duration::from_secs(30)));
        assert!(g.should_say(t0 + MIN_GAP));
        // Memory by 2 % of its total, CPU by 5 points, a disk appearing.
        let base = g.latest.clone();
        let mut m = base.clone();
        m.memory_available_bytes -= 2 * GB;
        assert!(m.moved_from(&base));
        let mut c = base.clone();
        c.cpu_busy_permille += 50;
        assert!(c.moved_from(&base));
        let mut d = base.clone();
        d.disks.push(v1::DiskUsage::default());
        assert!(d.moved_from(&base));
        assert!(!base.moved_from(&base));
    }
}
