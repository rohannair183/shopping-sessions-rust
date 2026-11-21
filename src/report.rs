use crate::{
    campaign::CampaignDays,
    model::{Click, Event, ORGANIC, Person, Timeline, UserRow},
    time::{DAY, VISIT_GAP, WEEK},
};
use std::collections::HashMap;

struct Visit {
    start: i64,
    day: i32,
    revenue: i64,
}
#[derive(Default)]
struct Daily {
    sessions: i64,
    events: i64,
    revenue: i64,
    by_category: HashMap<usize, i64>,
}

pub fn shopper(
    person: Person,
    mut timeline: Timeline,
    rows: &mut Vec<UserRow>,
    campaigns: &mut CampaignDays,
) {
    timeline.events.sort_unstable_by_key(|e| e.ts);
    let mut daily: HashMap<i32, Daily> = HashMap::new();
    let visits = build_visits(&timeline.events, &mut daily);
    for (day, stats) in daily {
        let top = stats
            .by_category
            .into_iter()
            .max_by(|(a, x), (b, y)| x.cmp(y).then_with(|| b.cmp(a)))
            .map(|(id, _)| id);
        rows.push(UserRow {
            person,
            day,
            sessions: stats.sessions,
            events: stats.events,
            revenue: stats.revenue,
            top,
        });
    }
    // Put the smallest click ID last at each timestamp so a forward scan retains it.
    timeline
        .clicks
        .sort_unstable_by(|a, b| a.ts.cmp(&b.ts).then_with(|| b.id.bytes().cmp(a.id.bytes())));
    timeline
        .clicks
        .dedup_by(|a, b| a.id.bytes() == b.id.bytes());
    credit(&visits, &timeline.clicks, campaigns);
    assists(&visits, &timeline.clicks, campaigns);
}

fn build_visits(events: &[Event], daily: &mut HashMap<i32, Daily>) -> Vec<Visit> {
    let mut visits: Vec<Visit> = Vec::new();
    let mut previous: Option<i64> = None;
    for event in events {
        if previous.is_none_or(|last| event.ts - last > VISIT_GAP) {
            visits.push(Visit {
                start: event.ts,
                day: event.ts.div_euclid(DAY) as i32,
                revenue: 0,
            });
            daily
                .entry(visits.last().unwrap().day)
                .or_default()
                .sessions += 1;
        }
        let visit = visits.last_mut().unwrap();
        let stats = daily.get_mut(&visit.day).unwrap();
        stats.events += 1;
        if event.cents > 0 {
            visit.revenue += event.cents;
            stats.revenue += event.cents;
            *stats.by_category.entry(event.category).or_default() += event.cents;
        }
        previous = Some(event.ts);
    }
    visits
}

fn credit(visits: &[Visit], clicks: &[Click], totals: &mut CampaignDays) {
    let mut next = 0;
    for visit in visits {
        while next < clicks.len() && clicks[next].ts <= visit.start {
            next += 1;
        }
        let campaign = next
            .checked_sub(1)
            .map(|i| &clicks[i])
            .filter(|c| visit.start - c.ts <= WEEK)
            .map_or(ORGANIC, |c| c.campaign);
        let stats = totals.get_mut(campaign, visit.day);
        stats.sessions += 1;
        stats.purchases += i64::from(visit.revenue > 0);
        stats.revenue += visit.revenue;
    }
}

fn assists(visits: &[Visit], clicks: &[Click], totals: &mut CampaignDays) {
    if clicks.is_empty() || visits.is_empty() {
        return;
    }
    let mut day_end = vec![0; visits.len()];
    let mut pos = 0;
    for group in visits.chunk_by(|a, b| a.day == b.day) {
        let end = pos + group.len();
        day_end[pos..end].fill(end);
        pos = end;
    }
    let mut counted: HashMap<usize, usize> = HashMap::new();
    let (mut left, mut right) = (0, 0);
    for c in clicks {
        while left < visits.len() && visits[left].start < c.ts {
            left += 1;
        }
        while right < visits.len() && visits[right].start <= c.ts + WEEK {
            right += 1;
        }
        // Only the new suffix can contribute. Older clicks of this campaign already counted the overlap.
        let mut add = left.max(counted.insert(c.campaign, right).unwrap_or(0));
        while add < right {
            let end = right.min(day_end[add]);
            totals.get_mut(c.campaign, visits[add].day).assists += (end - add) as i64;
            add = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_time_clicks_credit_the_smallest_original_id() {
        for (winner, runner_up) in [
            ("ka", "ka\0"),
            ("ka1234567", "ka12345670"),
            ("a123456789", "b"),
            ("K12345678", "k12345678"),
            ("éééééa", "éééééb"),
        ] {
            let timeline = Timeline {
                events: vec![Event {
                    ts: 0,
                    cents: 25,
                    category: 0,
                }],
                clicks: [(runner_up, 2), (winner, 1), (winner, 1)]
                    .map(|(id, campaign)| Click {
                        id: id.into(),
                        ts: 0,
                        campaign,
                    })
                    .into(),
            };
            let mut totals = CampaignDays::new(3, 0, DAY, 1);
            shopper(0, timeline, &mut Vec::new(), &mut totals);
            let totals: HashMap<_, _> = totals.into_rows().into_iter().collect();
            assert_eq!(totals[&(1, 0)].revenue, 25, "{winner:?} vs {runner_up:?}");
            assert_eq!(totals[&(2, 0)].sessions, 0);
            assert_eq!(totals[&(1, 0)].assists, 1);
            assert_eq!(totals[&(2, 0)].assists, 1);
        }
    }
}
