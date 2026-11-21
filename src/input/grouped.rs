use super::{Batches, EventId, Lookups, OwnedEvent, RepeatFilter, buckets, mix_id, regroup};
use crate::{
    Result,
    csv::chunks,
    identity::{DIRECT, People, direct_person, with_direct_name},
    model::{Event, Person},
    time::timestamp,
};
use rayon::prelude::*;
use std::{
    collections::HashMap,
    hash::{BuildHasherDefault, DefaultHasher, Hash, Hasher},
    io,
    path::Path,
    sync::atomic::{AtomicU32, Ordering},
};

const SIDE: u32 = 1 << 31;
const NO_PAYLOAD: u32 = u32::MAX;
const DEAD: u32 = u32::MAX;
const WIDE_EVENT: u32 = 0xff00_0000;
const PRICE_MASK: u32 = 0x00ff_ffff;
const ORDER_BIAS: i64 = 1 << 30;

#[derive(Clone, Copy)]
struct Location {
    order: u32,
    position: u32,
}

struct Version {
    key: u64,
    at: Location,
}

#[derive(Default)]
struct VersionBin {
    inline: Vec<Version>,
    text: Vec<(Box<str>, Location)>,
}

struct Payload {
    alias: usize,
    millis: u32,
    details: AtomicU32,
}

const _: () = assert!(size_of::<Version>() == 16);
const _: () = assert!(size_of::<Payload>() == 16);

struct EventChunk {
    versions: Vec<VersionBin>,
    payloads: Vec<Vec<Payload>>,
    aliases: Vec<Box<str>>,
    wide_events: Vec<Event>,
    wide_orders: Vec<i64>,
    wide_positions: Vec<(usize, usize)>,
    shift: u32,
}

impl EventChunk {
    fn new(shards: usize) -> Self {
        debug_assert!(shards.is_power_of_two());
        Self {
            versions: (0..shards).map(|_| VersionBin::default()).collect(),
            payloads: buckets(shards),
            aliases: Vec::new(),
            wide_events: Vec::new(),
            wide_orders: Vec::new(),
            wide_positions: Vec::new(),
            shift: shards.ilog2(),
        }
    }

    fn add_payload(&mut self, user: &str, event: Event, start: i64) -> u32 {
        let (alias, hash) = if let Some(key) = direct_person(user) {
            (key, mix_id(key as u64))
        } else {
            let index = self.aliases.len();
            self.aliases.push(user.into());
            let mut h = DefaultHasher::new();
            user.hash(&mut h);
            (index, h.finish())
        };
        let delta = event
            .ts
            .checked_sub(start)
            .and_then(|t| u32::try_from(t).ok());
        let (millis, details) = if let Some(t) = delta
            && event.category < 255
        {
            debug_assert!((0..=i64::from(PRICE_MASK)).contains(&event.cents));
            (t, ((event.category as u32) << 24) | event.cents as u32)
        } else {
            // Each chunk covers at most 4 MiB, too few rows to exhaust 24 bits.
            let index = self.wide_events.len();
            assert!(index < PRICE_MASK as usize);
            self.wide_events.push(event);
            (0, WIDE_EVENT | index as u32)
        };
        let shard = hash as usize & (self.payloads.len() - 1);
        let pos = self.payloads[shard].len();
        self.payloads[shard].push(Payload {
            alias,
            millis,
            details: AtomicU32::new(details),
        });
        if let Some(n) = pos
            .checked_mul(self.payloads.len())
            .and_then(|n| n.checked_add(shard))
            && n < SIDE as usize
        {
            n as u32
        } else {
            let index = self.wide_positions.len();
            self.wide_positions.push((shard, pos));
            SIDE | index as u32
        }
    }

    fn add(&mut self, id: &str, ingest: i64, user: &str, event: Option<Event>, start: i64) {
        let position = event.map_or(NO_PAYLOAD, |e| self.add_payload(user, e, start));
        let relative = ingest.div_euclid(1000) - start.div_euclid(1000) + ORDER_BIAS;
        let order = if ingest.rem_euclid(1000) == 0 && (0..i64::from(SIDE)).contains(&relative) {
            relative as u32
        } else {
            let index = self.wide_orders.len();
            self.wide_orders.push(ingest);
            SIDE | index as u32
        };
        let key = EventId::from(id);
        let shard = key.shard(self.versions.len());
        let at = Location { order, position };
        match key {
            EventId::Short(key) => self.versions[shard].inline.push(Version { key, at }),
            EventId::Text(key) => self.versions[shard].text.push((key, at)),
        }
    }

    fn ingest(&self, order: u32, start: i64) -> i64 {
        if order & SIDE == 0 {
            (start.div_euclid(1000) + i64::from(order) - ORDER_BIAS) * 1000
        } else {
            self.wide_orders[(order & !SIDE) as usize]
        }
    }

    fn mark_loser(&self, pos: u32) {
        if pos == NO_PAYLOAD {
            return;
        }
        let (bin, offset) = if pos & SIDE == 0 {
            (
                pos as usize & (self.payloads.len() - 1),
                (pos >> self.shift) as usize,
            )
        } else {
            self.wide_positions[(pos & !SIDE) as usize]
        };
        self.payloads[bin][offset]
            .details
            .store(DEAD, Ordering::Relaxed);
    }

    fn event(&self, p: &Payload, start: i64) -> Option<Event> {
        let details = p.details.load(Ordering::Relaxed);
        if details == DEAD {
            None
        } else if details & WIDE_EVENT == WIDE_EVENT {
            let wide = &self.wide_events[(details & PRICE_MASK) as usize];
            Some(Event {
                ts: wide.ts,
                cents: wide.cents,
                category: wide.category,
            })
        } else {
            Some(Event {
                ts: start + i64::from(p.millis),
                cents: i64::from(details & PRICE_MASK),
                category: (details >> 24) as usize,
            })
        }
    }
}

fn event_read(data: &Path, refs: &Lookups, shards: usize) -> io::Result<Vec<EventChunk>> {
    chunks(&data.join("events"))?
        .par_iter()
        .map(|chunk| {
            let mut part = EventChunk::new(shards);
            chunk.read_fields::<7>(|row| {
                if row[0].is_empty() {
                    return;
                }
                let Some(ingest) = timestamp(row[3]) else {
                    return;
                };
                part.add(row[0], ingest, row[1], refs.event(&row, ingest), refs.start);
            })?;
            Ok(part)
        })
        .collect()
}

fn remember<K: Eq + Hash>(
    earliest: &mut HashMap<K, (i64, usize, u32)>,
    key: K,
    part: usize,
    at: Location,
    chunks: &[EventChunk],
    start: i64,
) {
    use std::collections::hash_map::Entry;
    let time = chunks[part].ingest(at.order, start);
    match earliest.entry(key) {
        Entry::Vacant(e) => {
            e.insert((time, part, at.position));
        }
        Entry::Occupied(mut e) => {
            let (old, owner, pos) = *e.get();
            if time < old {
                chunks[owner].mark_loser(pos);
                e.insert((time, part, at.position));
            } else {
                chunks[part].mark_loser(at.position);
            }
        }
    }
}

fn remove_duplicates(bins: Vec<(usize, VersionBin)>, chunks: &[EventChunk], start: i64) {
    let mut filter = RepeatFilter::new(bins.iter().map(|(_, b)| b.inline.len()).sum());
    for (_, bin) in &bins {
        for v in &bin.inline {
            filter.record(v.key);
        }
    }
    let mut short = HashMap::new();
    let mut text = HashMap::new();
    for (part, bin) in bins {
        for v in bin.inline {
            if filter.might_repeat(v.key) {
                remember(&mut short, v.key, part, v.at, chunks, start);
            }
        }
        for (key, at) in bin.text {
            remember(&mut text, key, part, at, chunks, start);
        }
    }
}

#[derive(Default)]
struct AliasHash(u64);

impl Hasher for AliasHash {
    fn write(&mut self, b: &[u8]) {
        let mut h = DefaultHasher::new();
        h.write(b);
        self.0 = h.finish();
    }

    fn write_usize(&mut self, key: usize) {
        // Low bits of mix_id were already used to select the raw shopper shard.
        self.0 = mix_id(key as u64).rotate_right(32);
    }

    fn finish(&self) -> u64 {
        self.0
    }
}

fn route_shoppers(
    shard: usize,
    chunks: &[EventChunk],
    start: i64,
    groups: usize,
    mut resolve: impl FnMut(&str) -> Person,
) -> Vec<Vec<OwnedEvent>> {
    let mut inline = HashMap::<usize, Person, BuildHasherDefault<AliasHash>>::default();
    let mut text = HashMap::<&str, Person>::new();
    let mut out = buckets(groups);
    for chunk in chunks {
        for p in &chunk.payloads[shard] {
            let Some(event) = chunk.event(p, start) else {
                continue;
            };
            let person = if p.alias & DIRECT != 0 {
                *inline
                    .entry(p.alias)
                    .or_insert_with(|| with_direct_name(p.alias, &mut resolve))
            } else {
                let id = &*chunk.aliases[p.alias];
                *text.entry(id).or_insert_with(|| resolve(id))
            };
            out[person % groups].push((person, event));
        }
    }
    out
}

pub struct WinningEvents(Vec<EventChunk>);

pub fn read_winners(data: &Path, refs: &Lookups, groups: usize) -> Result<WinningEvents> {
    let shards = groups.next_power_of_two();
    let mut scanned = event_read(data, refs, shards)?;
    let mut versions = buckets(shards);
    for (owner, part) in scanned.iter_mut().enumerate() {
        for (shard, bin) in std::mem::take(&mut part.versions).into_iter().enumerate() {
            versions[shard].push((owner, bin));
        }
    }
    versions
        .into_par_iter()
        .for_each(|bins| remove_duplicates(bins, &scanned, refs.start));
    Ok(WinningEvents(scanned))
}

pub fn route_winners(
    winners: WinningEvents,
    groups: usize,
    refs: &Lookups,
    people: &People,
) -> Batches<OwnedEvent> {
    let scanned = winners.0;
    let shards = groups.next_power_of_two();
    let out = (0..shards)
        .into_par_iter()
        .map(|shard| route_shoppers(shard, &scanned, refs.start, groups, |id| people.resolve(id)))
        .collect();
    regroup(out, groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn detail(ts: i64, category: usize) -> Option<Event> {
        Some(Event {
            ts,
            category,
            cents: 10_000_000,
        })
    }

    #[test]
    fn mark_losing_payloads_across_shoppers_then_resolve_each_raw_alias_once() {
        let mut chunks = vec![EventChunk::new(1), EventChunk::new(1)];
        let start = 12_345;
        let wide = 8_000_000_000_000;
        let rare_alias = "\0long alias é";
        chunks[0].add("e00000001", 2000, "later", detail(start + 11, 0), start);
        chunks[1].add("e00000001", 1000, "broken", None, start);
        chunks[1].add("e00000002", 3000, "u1", detail(start + 12, 254), start);
        chunks[0].add("e00000003", 1000, "u1", detail(start + 13, 0), start);
        chunks[1].add("rare\0event", 1111, rare_alias, detail(wide, 300), start);
        chunks[0].add("rare\0event", wide, "other", detail(start + 1, 0), start);
        chunks[0].add("E00000003", 2000, rare_alias, detail(start + 14, 2), start);
        let bins: Vec<_> = chunks
            .iter_mut()
            .enumerate()
            .map(|(i, c)| (i, c.versions.pop().unwrap()))
            .collect();
        remove_duplicates(bins, &chunks, start);
        let mut lookups = Vec::new();
        let mut events: Vec<_> = route_shoppers(0, &chunks, start, 3, |name| {
            lookups.push(name.to_owned());
            2
        })
        .into_iter()
        .flatten()
        .map(|(_, e)| (e.ts, e.category, e.cents))
        .collect();
        lookups.sort_unstable();
        events.sort_unstable();
        assert_eq!(lookups, [rare_alias, "u1"]);
        assert_eq!(
            events,
            [
                (start + 12, 254, 10_000_000),
                (start + 13, 0, 10_000_000),
                (start + 14, 2, 10_000_000),
                (wide, 300, 10_000_000),
            ]
        );
    }

    #[test]
    fn compact_versions_keep_distinct_ids_that_share_a_filter_slot() {
        let mut part = EventChunk::new(1);
        let filter = RepeatFilter::new(2);
        let first = "e00000000";
        let EventId::Short(key) = EventId::from(first) else {
            unreachable!()
        };
        let colliding = (1..10_000)
            .map(|i| format!("e{i:08}"))
            .find(|id| {
                let EventId::Short(k) = EventId::from(id.as_str()) else {
                    unreachable!()
                };
                filter.slot(k) == filter.slot(key)
            })
            .unwrap();
        part.add(first, 1000, "A", detail(1000, 0), 0);
        part.add(&colliding, 1000, "B", detail(1000, 0), 0);
        let bins = vec![(0, part.versions.pop().unwrap())];
        let chunks = vec![part];
        remove_duplicates(bins, &chunks, 0);
        let out = route_shoppers(0, &chunks, 0, 1, |_| 0);
        assert_eq!(out[0].len(), 2);
    }
}
