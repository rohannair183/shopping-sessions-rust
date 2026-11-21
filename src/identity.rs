use crate::csv::{chunks, fields};
use std::{collections::HashMap, io, path::Path, sync::Arc};

#[derive(Default)]
struct Groups {
    ids: HashMap<Arc<str>, usize>,
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl Groups {
    fn index(&mut self, id: &str) -> usize {
        if let Some(&i) = self.ids.get(id) {
            return i;
        }
        let i = self.parent.len();
        self.ids.insert(Arc::from(id), i);
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

pub struct People(HashMap<Arc<str>, Arc<str>>);

impl People {
    pub fn read(folder: &Path) -> io::Result<Self> {
        let mut groups = Groups::default();
        for chunk in chunks(folder)? {
            chunk.read(|line| {
                if let Some([a, b]) = fields(line) {
                    if !a.is_empty() && !b.is_empty() {
                        groups.link(a, b);
                    }
                }
            })?;
        }
        let ids = std::mem::take(&mut groups.ids);
        let mut names: HashMap<usize, Arc<str>> = HashMap::new();
        for (id, &i) in &ids {
            let name = names.entry(groups.root(i)).or_insert_with(|| id.clone());
            if id < name {
                *name = id.clone();
            }
        }
        Ok(Self(
            ids.into_iter()
                .map(|(id, i)| (id, names[&groups.root(i)].clone()))
                .collect(),
        ))
    }
    pub fn name(&self, id: &str) -> Arc<str> {
        self.0.get(id).cloned().unwrap_or_else(|| Arc::from(id))
    }
}
