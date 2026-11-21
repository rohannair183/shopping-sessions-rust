use crate::{model::CampaignStats, time::DAY};
use std::collections::HashMap;

// The group reports and final merge share this budget for dense totals.
const DENSE_BUDGET: usize = 64 * 1024 * 1024;
pub type CampaignRow = ((usize, i32), CampaignStats);

pub enum CampaignDays {
    Dense {
        first_day: i32,
        days: usize,
        slots: Vec<CampaignStats>,
    },
    Sparse(HashMap<(usize, i32), CampaignStats>),
}

impl CampaignDays {
    pub fn new(campaigns: usize, start: i64, end: i64, groups: usize) -> Self {
        let first = start.div_euclid(DAY);
        let last = (end - 1).div_euclid(DAY);
        let max_slots = DENSE_BUDGET / size_of::<CampaignStats>() / groups.saturating_add(1);
        if let (Ok(first_day), Ok(_)) = (i32::try_from(first), i32::try_from(last))
            && let Ok(days) = usize::try_from(last - first + 1)
            && let Some(count) = days.checked_mul(campaigns)
            && count <= max_slots
        {
            return Self::Dense {
                first_day,
                days,
                slots: vec![CampaignStats::default(); count],
            };
        }
        Self::Sparse(HashMap::new())
    }

    pub fn get_mut(&mut self, campaign: usize, day: i32) -> &mut CampaignStats {
        match self {
            Self::Dense {
                first_day,
                days,
                slots,
            } => &mut slots[campaign * *days + (day - *first_day) as usize],
            Self::Sparse(rows) => rows.entry((campaign, day)).or_default(),
        }
    }

    pub fn merge(&mut self, part: Self) {
        match (self, part) {
            (Self::Dense { slots, .. }, Self::Dense { slots: part, .. }) => {
                for (total, stats) in slots.iter_mut().zip(part) {
                    total.add(stats);
                }
            }
            (Self::Sparse(rows), Self::Sparse(part)) => {
                for (key, stats) in part {
                    rows.entry(key).or_default().add(stats);
                }
            }
            _ => unreachable!("All groups use the same catalog, window and memory budget"),
        }
    }

    pub fn into_rows(self) -> Vec<CampaignRow> {
        match self {
            Self::Dense {
                first_day,
                days,
                slots,
            } => slots
                .into_iter()
                .enumerate()
                .filter(|(_, s)| s.sessions > 0 || s.assists > 0)
                .map(|(i, s)| ((i / days, first_day + (i % days) as i32), s))
                .collect(),
            Self::Sparse(rows) => rows.into_iter().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dense_and_sparse_keep_only_credited_or_assisted_days_after_merge() {
        // The half-open window crosses the epoch and ends at midnight on day 2.
        for campaigns in [3, usize::MAX] {
            let new = || CampaignDays::new(campaigns, -1, 2 * DAY, 24);
            let mut a = new();
            assert_eq!(matches!(a, CampaignDays::Dense { .. }), campaigns == 3);
            a.get_mut(0, -1).sessions = 1;
            a.get_mut(2, 1).assists = 2;
            let mut b = new();
            b.get_mut(0, -1).sessions = 2;
            b.get_mut(1, 0).purchases = 1;
            b.get_mut(1, 0).revenue = 200;
            b.get_mut(1, 0).sessions = 1;
            a.merge(b);
            let mut result: Vec<_> = a
                .into_rows()
                .into_iter()
                .map(|(key, s)| (key, s.sessions, s.purchases, s.revenue, s.assists))
                .collect();
            result.sort_unstable();
            assert_eq!(
                result,
                [
                    ((0, -1), 3, 0, 0, 0),
                    ((1, 0), 1, 1, 200, 0),
                    ((2, 1), 0, 0, 0, 2)
                ]
            );
        }
    }
}
