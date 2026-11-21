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

#[derive(Default)]
struct Groups {
    ids: HashMap<Box<str>, usize>,
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl Groups {
    fn index(&mut self, id: &str) -> usize {
        if let Some(&i) = self.ids.get(id) {
            return i;
        }
        let i = self.parent.len();
        self.ids.insert(id.into(), i);
        self.parent.push(i);
        self.size.push(1);
        i
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
    linked: HashMap<Box<str>, Person>,
    names: Vec<Box<str>>,
    unlinked: [Mutex<HashMap<Box<str>, Person>>; SHARDS],
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
        let mut roots = vec![None; linked.len()];
        let mut names = Vec::new();
        for (id, index) in &mut linked {
            let root = groups.root(*index);
            let person = *roots[root].get_or_insert_with(|| {
                names.push(id.clone());
                names.len() - 1
            });
            if id < &names[person] {
                names[person] = id.clone();
            }
            *index = person;
        }
        Self {
            next: AtomicUsize::new(names.len()),
            linked,
            names,
            unlinked: std::array::from_fn(|_| Mutex::default()),
            hasher: RandomState::new(),
        }
    }

    pub fn resolve(&self, id: &str) -> Person {
        if let Some(&person) = self.linked.get(id) {
            return person;
        }
        // All workers registering the same unlinked ID must choose the same shard.
        let shard = self.hasher.hash_one(id) as usize % SHARDS;
        let mut ids = self.unlinked[shard].lock().unwrap();
        if let Some(&person) = ids.get(id) {
            return person;
        }
        let person = self.next.fetch_add(1, Ordering::Relaxed);
        ids.insert(id.into(), person);
        person
    }

    pub fn into_names(self) -> Names {
        let mut names = self.names;
        names.resize_with(self.next.into_inner(), Box::<str>::default);
        for shard in self.unlinked {
            for (id, person) in shard.into_inner().unwrap() {
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
}
