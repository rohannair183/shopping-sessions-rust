use crate::{
    csv::{chunks, fields},
    model::Person,
};
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

#[derive(Default)]
struct Groups {
    ids: Aliases,
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl Groups {
    fn index(&mut self, id: &str) -> usize {
        let next = self.parent.len();
        let index = self.ids.intern(Key::from(id), || next);
        if index == next {
            self.parent.push(next);
            self.size.push(1);
        }
        index
    }
    fn root(&mut self, mut i: usize) -> usize {
        while self.parent[i] != i {
            self.parent[i] = self.parent[self.parent[i]];
            i = self.parent[i];
        }
        i
    }
    fn link(&mut self, left: &str, right: &str) {
        let a = self.index(left);
        let b = self.index(right);
        let mut a = self.root(a);
        let mut b = self.root(b);
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

pub struct People {
    linked: Aliases,
    names: Vec<Box<str>>,
    unlinked: [Mutex<Aliases>; SHARDS],
    hasher: RandomState,
    next: AtomicUsize,
}

pub struct Names(Vec<Box<str>>);

impl Names {
    pub fn get(&self, person: Person) -> &str {
        &self.0[person]
    }
}

impl People {
    pub fn read(folder: &Path) -> io::Result<Self> {
        let mut groups = Groups::default();
        for chunk in chunks(folder)? {
            chunk.read(|line| {
                if let Some([a, b]) = fields(line)
                    && !a.is_empty()
                    && !b.is_empty()
                {
                    groups.link(a, b);
                }
            })?;
        }
        Ok(Self::from_groups(groups))
    }

    fn from_groups(mut groups: Groups) -> Self {
        let mut linked = std::mem::take(&mut groups.ids);
        let mut roots = vec![None; groups.parent.len()];
        let mut names = Vec::new();
        for (id, index) in linked.entries() {
            let root = groups.root(*index);
            let person = *roots[root].get_or_insert_with(|| {
                names.push(id);
                names.len() - 1
            });
            names[person] = names[person].min(id);
            *index = person;
        }
        let names: Vec<Box<str>> = names.into_iter().map(Box::from).collect();
        Self {
            next: AtomicUsize::new(names.len()),
            linked,
            names,
            unlinked: std::array::from_fn(|_| Mutex::default()),
            hasher: RandomState::new(),
        }
    }

    pub fn resolve(&self, id: &str) -> Person {
        let key = Key::from(id);
        if let Some(person) = self.linked.get(key) {
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
        let mut groups = Groups::default();
        groups.link("u12345678", "alias");
        groups.link("alias", " unseen");
        let people = People::from_groups(groups);
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
            results[0].map(|person| names.get(person)),
            [" unseen", " unseen", "é", "É", "é "]
        );
    }

    #[test]
    fn inline_names_round_trip_as_utf8_and_mix_with_text_aliases() {
        let mut groups = Groups::default();
        groups.link("alias longer than nine bytes", "A12345678");
        let people = People::from_groups(groups);
        let canonical = people.resolve("alias longer than nine bytes");
        assert_eq!(canonical, people.resolve("A12345678"));
        let inputs = ["é1234567", "É1234567", "é1234567 ", "a12345678"];
        let ids = inputs.map(|id| people.resolve(id));
        let names = people.into_names();
        assert_eq!(names.get(canonical), "A12345678");
        assert_eq!(ids.map(|id| names.get(id)), inputs);
    }
}
