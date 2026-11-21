use crate::{
    Result,
    csv::{Chunk, chunks, fields},
    identity::People,
    model::{Click, Event, Person},
    time::{DAY, WEEK, number, timestamp},
};
use rayon::prelude::*;
use std::{
    collections::HashMap,
    fs,
    hash::{DefaultHasher, Hash, Hasher},
    io,
    path::Path,
};

pub struct Lookups {
    pub categories: Vec<String>,
    pub campaigns: Vec<(String, String)>,
    products: HashMap<String, usize>,
    campaign_ids: HashMap<String, usize>,
    unknown: usize,
    start: i64,
    end: i64,
}

fn pairs(path: &Path) -> io::Result<Vec<(String, String)>> {
    let mut rows = Vec::new();
    Chunk::whole(path.to_owned())?.read(|line| {
        if let Some([key, value]) = fields(line) {
            rows.push((key.into(), value.into()));
        }
    })?;
    Ok(rows)
}

impl Lookups {
    pub fn read(data: &Path) -> Result<Self> {
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(data.join("manifest.json"))?)?;
        let bound = |key: &str| {
            manifest[key]
                .as_str()
                .and_then(timestamp)
                .ok_or_else(|| format!("Missing or invalid {key} in manifest.json"))
        };
        let (start, end) = (bound("window_start")?, bound("window_end")?);
        if start >= end {
            return Err("Empty manifest window".into());
        }
        let rows = pairs(&data.join("products.csv"))?;
        let mut categories: Vec<String> = rows.iter().map(|(_, v)| v.clone()).collect();
        categories.push("unknown".into());
        categories.sort_unstable();
        categories.dedup();
        let cat = |v: &str| categories.binary_search_by(|s| s.as_str().cmp(v)).unwrap();
        let unknown = cat("unknown");
        let products = rows.into_iter().map(|(p, c)| (p, cat(&c))).collect();
        let mut campaigns = vec![("organic".into(), "organic".into())];
        campaigns.extend(pairs(&data.join("campaigns.csv"))?);
        let campaign_ids = campaigns
            .iter()
            .enumerate()
            .skip(1)
            .map(|(i, (c, _))| (c.clone(), i))
            .collect();
        Ok(Self {
            categories,
            campaigns,
            products,
            campaign_ids,
            unknown,
            start,
            end,
        })
    }

    fn event(&self, row: &[&str; 7], ingest: i64) -> Option<Event> {
        let ts = timestamp(row[2])?;
        if row[1].is_empty()
            || row[5].is_empty()
            || !(self.start..self.end).contains(&ts)
            || ingest - ts > DAY
        {
            return None;
        }
        let (cents, category) = match row[4] {
            "view" | "cart" => (0, self.unknown),
            "purchase" => {
                let amount = number(row[6].as_bytes())?;
                if !(1..=10_000_000).contains(&amount) {
                    return None;
                }
                (amount, *self.products.get(row[5]).unwrap_or(&self.unknown))
            }
            _ => return None,
        };
        Some(Event {
            ts,
            cents,
            category,
        })
    }
}

type Batches<T> = Vec<Vec<Vec<T>>>;
type OwnedEvent = (Person, Event);
type OwnedClick = (Person, Click);

#[derive(PartialEq, Eq, Hash)]
enum EventId {
    Short(u64),
    Text(Box<str>),
}

impl From<&str> for EventId {
    fn from(id: &str) -> Self {
        if let Some(tail) = id.strip_prefix('e')
            && let Ok(bytes) = tail.as_bytes().try_into()
        {
            return Self::Short(u64::from_le_bytes(bytes));
        }
        Self::Text(id.into())
    }
}

struct Candidate {
    id: EventId,
    ingest: i64,
    payload: Option<OwnedEvent>,
}

fn shard(key: &str, count: usize) -> usize {
    let mut h = DefaultHasher::new();
    key.hash(&mut h);
    h.finish() as usize % count
}
fn buckets<T>(n: usize) -> Vec<Vec<T>> {
    (0..n).map(|_| Vec::new()).collect()
}

// Move vectors between queues without copying their records.
fn regroup<T>(batches: Batches<T>, count: usize) -> Batches<T> {
    let mut groups = buckets(count);
    for batch in batches {
        for (group, part) in groups.iter_mut().zip(batch) {
            if !part.is_empty() {
                group.push(part);
            }
        }
    }
    groups
}

pub fn events(
    data: &Path,
    people: &People,
    refs: &Lookups,
    count: usize,
) -> Result<Batches<OwnedEvent>> {
    let batches: Batches<Candidate> = chunks(&data.join("events"))?
        .par_iter()
        .map(|chunk| {
            let mut out = buckets(count);
            chunk.read(|line| {
                let Some(row) = fields::<7>(line) else {
                    return;
                };
                if row[0].is_empty() {
                    return;
                }
                let Some(ingest) = timestamp(row[3]) else {
                    return;
                };
                // Invalid copies still compete for earliest ingestion.
                let payload = refs
                    .event(&row, ingest)
                    .map(|event| (people.resolve(row[1]), event));
                out[shard(row[0], count)].push(Candidate {
                    id: row[0].into(),
                    ingest,
                    payload,
                });
            })?;
            Ok::<_, io::Error>(out)
        })
        .collect::<io::Result<_>>()?;
    let winners = regroup(batches, count)
        .into_par_iter()
        .map(|parts| {
            let mut earliest = HashMap::new();
            for c in parts.into_iter().flatten() {
                let slot = earliest.entry(c.id).or_insert((i64::MAX, None));
                if c.ingest < slot.0 {
                    *slot = (c.ingest, c.payload);
                }
            }
            let mut out = buckets(count);
            for (_, (_, p)) in earliest {
                if let Some((person, event)) = p {
                    out[person % count].push((person, event));
                }
            }
            out
        })
        .collect();
    Ok(regroup(winners, count))
}

pub fn clicks(
    data: &Path,
    people: &People,
    refs: &Lookups,
    count: usize,
) -> Result<Batches<OwnedClick>> {
    let batches = chunks(&data.join("clicks"))?
        .par_iter()
        .map(|chunk| {
            let mut out = buckets(count);
            chunk.read(|line| {
                let Some([id, user, camp, ts]) = fields(line) else {
                    return;
                };
                if id.is_empty() || user.is_empty() {
                    return;
                }
                let (Some(&campaign), Some(ts)) = (refs.campaign_ids.get(camp), timestamp(ts))
                else {
                    return;
                };
                if !(refs.start - WEEK..refs.end).contains(&ts) {
                    return;
                }
                let person = people.resolve(user);
                out[person % count].push((
                    person,
                    Click {
                        id: id.into(),
                        ts,
                        campaign,
                    },
                ));
            })?;
            Ok::<_, io::Error>(out)
        })
        .collect::<io::Result<_>>()?;
    Ok(regroup(batches, count))
}

#[cfg(test)]
mod tests {
    use super::EventId;
    use std::collections::HashSet;

    #[test]
    fn duplicate_checks_preserve_every_byte_of_the_event_id() {
        let ids = [
            "e12345678",
            "e12345678",
            "E12345678",
            "12345678",
            "e12345678 ",
            "e1234567",
            "e123456é",
            "e123456É",
        ];
        let keys: HashSet<_> = ids.into_iter().map(EventId::from).collect();
        assert_eq!(keys.len(), 7);
    }
}
