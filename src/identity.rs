use crate::{
    csv::{chunks, fields},
    model::Person,
};
use rayon::prelude::*;
use std::{
    collections::HashMap,
    hash::{BuildHasher, RandomState},
    io,
    path::Path,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

// The common nine-byte shopper IDs fit inside a map key. Other lengths retain their full text.
const INLINE_LEN: usize = 9;

#[derive(Clone, Copy)]
enum Key<'a> {
    Inline(&'a [u8; INLINE_LEN]),
    Text(&'a str),
}

impl<'a> From<&'a str> for Key<'a> {
    fn from(id: &'a str) -> Self {
        match id.as_bytes().try_into() {
            Ok(bytes) => Self::Inline(bytes),
            Err(_) => Self::Text(id),
        }
    }
}

#[derive(Default)]
struct Aliases {
    inline: HashMap<[u8; INLINE_LEN], Person>,
    text: HashMap<Box<str>, Person>,
}

impl Aliases {
    fn get(&self, key: Key<'_>) -> Option<Person> {
        match key {
            Key::Inline(bytes) => self.inline.get(bytes).copied(),
            Key::Text(id) => self.text.get(id).copied(),
        }
    }

    fn intern(&mut self, key: Key<'_>, assign: impl FnOnce() -> Person) -> Person {
        match key {
            Key::Inline(bytes) => *self.inline.entry(*bytes).or_insert_with(assign),
            Key::Text(id) => {
                if let Some(&person) = self.text.get(id) {
                    return person;
                }
                let person = assign();
                self.text.insert(id.into(), person);
                person
            }
        }
    }

    fn entries(&mut self) -> impl Iterator<Item = (&str, &mut Person)> {
        self.inline
            .iter_mut()
            .map(|(bytes, person)| (std::str::from_utf8(bytes).unwrap(), person))
            .chain(
                self.text
                    .iter_mut()
                    .map(|(id, person)| (id.as_ref(), person)),
            )
    }
}

enum OwnedAlias {
    Inline([u8; INLINE_LEN]),
    Text(Box<str>),
}

impl OwnedAlias {
    fn new(id: &str) -> Self {
        match Key::from(id) {
            Key::Inline(bytes) => Self::Inline(*bytes),
            Key::Text(id) => Self::Text(id.into()),
        }
    }

    fn key(&self) -> Key<'_> {
        match self {
            Self::Inline(bytes) => Key::Inline(bytes),
            Self::Text(id) => Key::Text(id),
        }
    }
}

const LINK_PARTS: usize = 32;
const _: () = assert!(LINK_PARTS.is_power_of_two() && LINK_PARTS > 1);

fn link_part(id: &str) -> usize {
    use std::hash::{DefaultHasher, Hash, Hasher};
    const MIX: u64 = 0x9e3779b97f4a7c15;
    let hash = if let Ok(bytes) = <&[u8; INLINE_LEN]>::try_from(id.as_bytes()) {
        let tail = u64::from_le_bytes(bytes[1..].try_into().unwrap());
        let bits = tail ^ u64::from(bytes[0]).wrapping_mul(MIX);
        (bits ^ (bits >> 32)).wrapping_mul(MIX)
    } else {
        let mut h = DefaultHasher::new();
        id.hash(&mut h);
        h.finish()
    };
    (hash >> (64 - LINK_PARTS.ilog2())) as usize
}

#[derive(Default)]
struct LinkBatch {
    rows: usize,
    endpoints: [Vec<(OwnedAlias, usize)>; LINK_PARTS],
}

impl LinkBatch {
    fn link(&mut self, left: &str, right: &str) {
        for id in [left, right] {
            self.endpoints[link_part(id)].push((OwnedAlias::new(id), self.rows));
        }
        self.rows += 1;
    }
}

type LinkPiece = (usize, Vec<(OwnedAlias, usize)>);

fn index_link_partition(pieces: Vec<LinkPiece>) -> (Aliases, Vec<(usize, usize)>) {
    let mut aliases = Aliases::default();
    let mut joins = Vec::new();
    for (offset, endpoints) in pieces {
        for (id, local) in endpoints {
            let edge = offset + local;
            let first = aliases.intern(id.key(), || edge);
            if first != edge {
                joins.push((first, edge));
            }
        }
    }
    (aliases, joins)
}

struct Forest {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl Forest {
    fn new(edges: usize) -> Self {
        Self {
            parent: (0..edges).collect(),
            size: vec![1; edges],
        }
    }

    fn root(&mut self, mut edge: usize) -> usize {
        while self.parent[edge] != edge {
            self.parent[edge] = self.parent[self.parent[edge]];
            edge = self.parent[edge];
        }
        edge
    }

    fn join(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.root(a), self.root(b));
        if a == b {
            return;
        }
        if self.size[a] < self.size[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parent[b] = a;
        self.size[a] += self.size[b];
    }
}

const SHARDS: usize = 64;
pub(crate) const DIRECT: Person = 1 << (usize::BITS - 1);
const ASCII_BITS: u32 = 7;
const MAX_DIRECT_LEN: usize = ((usize::BITS - 1) / ASCII_BITS) as usize;
const ASCII_MASK: usize = (1 << ASCII_BITS) - 1;

// A nonzero first byte lets the decoder recover the original length, including trailing NULs.
pub(crate) fn direct_person(id: &str) -> Option<Person> {
    let bytes = id.as_bytes();
    if bytes.len() > MAX_DIRECT_LEN || bytes.first().is_none_or(|&b| b == 0) || !bytes.is_ascii() {
        return None;
    }
    let bits = bytes
        .iter()
        .fold(0, |bits, &b| (bits << ASCII_BITS) | usize::from(b));
    Some(DIRECT | bits)
}

pub struct People {
    linked: [Aliases; LINK_PARTS],
    names: Vec<Box<str>>,
    unlinked: [Mutex<Aliases>; SHARDS],
    hasher: RandomState,
    next: AtomicUsize,
}

pub struct Names(Vec<Box<str>>);

impl Names {
    pub fn with_name<T>(&self, person: Person, use_name: impl FnOnce(&str) -> T) -> T {
        if person & DIRECT == 0 {
            return use_name(&self.0[person]);
        }
        with_direct_name(person, use_name)
    }
}

pub(crate) fn with_direct_name<T>(person: Person, use_name: impl FnOnce(&str) -> T) -> T {
    let mut bytes = [0u8; MAX_DIRECT_LEN];
    let mut pos = bytes.len();
    let mut bits = person & !DIRECT;
    while bits != 0 {
        pos -= 1;
        bytes[pos] = (bits & ASCII_MASK) as u8;
        bits >>= ASCII_BITS;
    }
    use_name(std::str::from_utf8(&bytes[pos..]).unwrap())
}

impl People {
    pub fn read(folder: &Path) -> io::Result<Self> {
        let batches = chunks(folder)?
            .par_iter()
            .map(|chunk| {
                let mut batch = LinkBatch::default();
                chunk.read(|line| {
                    if let Some([a, b]) = fields(line)
                        && !a.is_empty()
                        && !b.is_empty()
                    {
                        batch.link(a, b);
                    }
                })?;
                Ok::<_, io::Error>(batch)
            })
            .collect::<io::Result<Vec<_>>>()?;
        Ok(Self::from_batches(batches))
    }

    fn from_batches(batches: Vec<LinkBatch>) -> Self {
        let mut partitions: [_; LINK_PARTS] = std::array::from_fn(|_| Vec::new());
        let mut total = 0;
        for batch in batches {
            for (part, endpoints) in partitions.iter_mut().zip(batch.endpoints) {
                part.push((total, endpoints));
            }
            total += batch.rows;
        }

        // Every occurrence of an alias reaches the same partition, even across files.
        let built: Vec<_> = partitions
            .into_par_iter()
            .map(index_link_partition)
            .collect();

        let mut forest = Forest::new(total);
        let mut linked: Vec<_> = built
            .into_iter()
            .map(|(aliases, joins)| {
                for (a, b) in joins {
                    forest.join(a, b);
                }
                aliases
            })
            .collect();
        let mut root_person = vec![usize::MAX; total];
        let mut persons = 0;
        let by_edge: Vec<_> = (0..total)
            .map(|edge| {
                let slot = &mut root_person[forest.root(edge)];
                if *slot == usize::MAX {
                    *slot = persons;
                    persons += 1;
                }
                *slot
            })
            .collect();
        let mut names = vec![""; persons];
        for part in &mut linked {
            for (id, edge) in part.entries() {
                *edge = by_edge[*edge];
                let smallest = &mut names[*edge];
                if smallest.is_empty() || id < *smallest {
                    *smallest = id;
                }
            }
        }
        let names = names.into_iter().map(Box::from).collect();
        Self {
            linked: linked.try_into().unwrap_or_else(|_| unreachable!()),
            names,
            next: AtomicUsize::new(persons),
            unlinked: std::array::from_fn(|_| Mutex::default()),
            hasher: RandomState::new(),
        }
    }

    pub fn resolve(&self, id: &str) -> Person {
        let key = Key::from(id);
        if let Some(person) = self.linked[link_part(id)].get(key) {
            return person;
        }
        if let Some(person) = direct_person(id) {
            return person;
        }
        // All workers registering the same unlinked ID must choose the same shard.
        let shard = self.hasher.hash_one(id) as usize % SHARDS;
        self.unlinked[shard]
            .lock()
            .unwrap()
            .intern(key, || self.next.fetch_add(1, Ordering::Relaxed))
    }

    pub fn into_names(self) -> Names {
        let mut names = self.names;
        names.resize_with(self.next.into_inner(), Box::<str>::default);
        for shard in self.unlinked {
            let ids = shard.into_inner().unwrap();
            for (bytes, person) in ids.inline {
                names[person] = std::str::from_utf8(&bytes).unwrap().into();
            }
            for (id, person) in ids.text {
                names[person] = id;
            }
        }
        Names(names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::Barrier, thread};

    #[test]
    fn concurrent_readers_share_numbers_and_preserve_output_names() {
        let mut groups = LinkBatch::default();
        groups.link("u12345678", "alias");
        groups.link("alias", " unseen");
        let people = People::from_batches(vec![groups]);
        let input = ["alias", "u12345678", "é", "É", "é "];
        let workers = 8;
        let start = Barrier::new(workers);
        let results = thread::scope(|scope| {
            let jobs: Vec<_> = (0..workers)
                .map(|_| {
                    scope.spawn(|| {
                        start.wait();
                        input.map(|id| people.resolve(id))
                    })
                })
                .collect();
            jobs.into_iter()
                .map(|job| job.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert!(results.iter().all(|ids| ids == &results[0]));
        assert_eq!(results[0][0], results[0][1]);
        let names = people.into_names();
        assert_eq!(
            results[0].map(|person| names.with_name(person, str::to_owned)),
            [" unseen", " unseen", "é", "É", "é "]
        );
    }

    #[test]
    fn inline_names_round_trip_as_utf8_and_mix_with_text_aliases() {
        let mut groups = LinkBatch::default();
        groups.link("alias longer than nine bytes", "A12345678");
        let people = People::from_batches(vec![groups]);
        let canonical = people.resolve("alias longer than nine bytes");
        assert_eq!(canonical, people.resolve("A12345678"));
        let inputs = ["é1234567", "É1234567", "é1234567 ", "a12345678"];
        let ids = inputs.map(|id| people.resolve(id));
        let names = people.into_names();
        assert_eq!(names.with_name(canonical, str::to_owned), "A12345678");
        assert_eq!(ids.map(|id| names.with_name(id, str::to_owned)), inputs);
    }

    #[test]
    fn direct_and_registered_names_remain_distinct() {
        let people = People::from_batches(vec![]);
        let inputs = [
            "u12345678",
            "U12345678",
            "u",
            "u\0",
            "\0u",
            "é",
            "u123456789",
            " u ",
        ];
        let ids = inputs.map(|s| people.resolve(s));
        assert_eq!(
            ids.iter().collect::<std::collections::HashSet<_>>().len(),
            inputs.len()
        );
        assert_ne!(people.resolve("u") & DIRECT, 0);
        assert_eq!(people.resolve("\0u") & DIRECT, 0);
        let names = people.into_names();
        assert_eq!(ids.map(|id| names.with_name(id, str::to_owned)), inputs);
    }

    #[test]
    fn transitive_linking_uses_the_smallest_original_name() {
        let mut left = LinkBatch::default();
        left.link(" z", "z");
        left.link("B12345678", "bridge");
        let mut right = LinkBatch::default();
        right.link("z", "two devices");
        right.link("two devices", "bridge");
        right.link("bridge", "bridge");
        right.link("B12345678", "bridge");
        right.link("É", "é");
        let people = People::from_batches(vec![left, LinkBatch::default(), right]);
        let household = people.resolve("z");
        for id in [" z", "two devices", "bridge", "B12345678"] {
            assert_eq!(people.resolve(id), household);
        }
        let other = people.resolve("É");
        assert_ne!(household, other);
        assert_eq!(people.resolve("é"), other);
        let names = people.into_names();
        assert_eq!(names.with_name(household, str::to_owned), " z");
        assert_eq!(names.with_name(other, str::to_owned), "É");
    }
}
