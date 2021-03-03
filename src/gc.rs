use std::{
    fmt, result,
    sync::{
        atomic::{AtomicPtr, AtomicU64, AtomicUsize, Ordering::SeqCst},
        Arc,
    },
};

use crate::{map::Child, map::Item, map::Node};

// pub const EPOCH_PERIOD: time::Duration = time::Duration::from_millis(10);
pub const ENTER_MASK: u64 = 0x8000000000000000;
pub const EPOCH_MASK: u64 = 0x7FFFFFFFFFFFFFFF;
pub const MAX_POOL_SIZE: usize = 1024;

// CAS operation

pub struct Epoch {
    epoch: Arc<AtomicU64>,
    at: Arc<AtomicU64>,
    n_compacts: Arc<AtomicUsize>,
    n_retries: Arc<AtomicUsize>,
}

impl Epoch {
    pub fn new(
        epoch: Arc<AtomicU64>,
        at: Arc<AtomicU64>,
        n_compacts: Arc<AtomicUsize>,
        n_retries: Arc<AtomicUsize>,
    ) -> Epoch {
        at.store(epoch.load(SeqCst) | ENTER_MASK, SeqCst);
        Epoch {
            epoch,
            at,
            n_compacts,
            n_retries,
        }
    }

    pub fn count_retries(&self, retries: usize) {
        match retries {
            0 | 1 => (),
            _ => {
                self.n_retries.fetch_add(1, SeqCst);
            }
        }
    }

    pub fn count_compacts(&self) {
        self.n_compacts.fetch_add(1, SeqCst);
    }
}

impl Drop for Epoch {
    fn drop(&mut self) {
        self.at.store(self.epoch.load(SeqCst), SeqCst);
        self.epoch.fetch_add(1, SeqCst);
    }
}

pub struct Cas<K, V> {
    reclaims: Vec<Box<Reclaim<K, V>>>,
    older: Vec<OwnedMem<K, V>>,
    newer: Vec<OwnedMem<K, V>>,

    child_pool: Vec<Box<Child<K, V>>>,
    node_trie_pool: Vec<Box<Node<K, V>>>,
    node_list_pool: Vec<Box<Node<K, V>>>,
    node_tomb_pool: Vec<Box<Node<K, V>>>,
    reclaim_pool: Vec<Box<Reclaim<K, V>>>,

    n_allocs: usize,
    n_frees: usize,
}

impl<K, V> Drop for Cas<K, V> {
    fn drop(&mut self) {
        debug_assert!(
            self.older.len() == 0,
            "invariant Cas::older should be ZERO on drop"
        );
        debug_assert!(
            self.newer.len() == 0,
            "invariant Cas::newer should be ZERO on drop"
        );
        debug_assert!(
            self.reclaims.len() == 0,
            "invariant Cas::reclaims should be ZERO on drop"
        );

        //#[cfg(test)]
        //println!(
        //    "Dropping Cas pools:reclaims:{}, pools:({},{},{},{},{}) allocs:{}/{}",
        //    self.reclaims.len(),
        //    self.child_pool.len(),
        //    self.node_trie_pool.len(),
        //    self.node_list_pool.len(),
        //    self.node_tomb_pool.len(),
        //    self.reclaim_pool.len(),
        //    self.n_allocs,
        //    self.n_frees
        //);
    }
}

impl<K, V> Cas<K, V> {
    pub fn new() -> Self {
        Cas {
            reclaims: Vec::with_capacity(64),
            older: Vec::with_capacity(64),
            newer: Vec::with_capacity(64),

            child_pool: Vec::with_capacity(64),
            node_trie_pool: Vec::with_capacity(64),
            node_list_pool: Vec::with_capacity(64),
            node_tomb_pool: Vec::with_capacity(64),
            reclaim_pool: Vec::with_capacity(64),

            n_allocs: 0,
            n_frees: 0,
        }
    }

    pub fn to_pools_len(&self) -> usize {
        self.child_pool.len()
            + self.node_trie_pool.len()
            + self.node_list_pool.len()
            + self.node_tomb_pool.len()
            + self.reclaim_pool.len()
    }

    pub fn to_alloc_count(&self) -> usize {
        self.n_allocs
    }

    pub fn to_free_count(&self) -> usize {
        self.n_frees
    }

    pub fn has_reclaims(&self) -> bool {
        self.reclaims.len() > 0
    }

    pub fn free_on_pass(&mut self, m: Mem<K, V>) {
        match m {
            Mem::Child(ptr) => unsafe {
                self.older.push(OwnedMem::Child(Box::from_raw(ptr)));
            },
            Mem::Node(ptr) => unsafe {
                self.older.push(OwnedMem::Node(Box::from_raw(ptr)));
            },
        }
    }

    pub fn free_on_fail(&mut self, m: Mem<K, V>) {
        match m {
            Mem::Child(ptr) => unsafe {
                self.newer.push(OwnedMem::Child(Box::from_raw(ptr)));
            },
            Mem::Node(ptr) => unsafe {
                self.newer.push(OwnedMem::Node(Box::from_raw(ptr)));
            },
        }
    }

    pub fn alloc_node(&mut self, variant: char) -> Box<Node<K, V>>
    where
        K: Default,
    {
        match variant {
            'l' => match self.node_list_pool.pop() {
                Some(val) => val,
                None => {
                    self.n_allocs += 1;
                    Box::new(Node::List {
                        items: Vec::with_capacity(2), // **IMPORTANT**
                    })
                }
            },
            't' => match self.node_trie_pool.pop() {
                Some(val) => val,
                None => {
                    self.n_allocs += 1;
                    Box::new(Node::Trie {
                        bmp: 0,
                        childs: Vec::with_capacity(1),
                    })
                }
            },
            'b' => match self.node_tomb_pool.pop() {
                Some(val) => val,
                None => {
                    self.n_allocs += 1;
                    Box::new(Node::Tomb {
                        item: Item::default(),
                    })
                }
            },
            _ => unreachable!(),
        }
    }

    pub fn alloc_child(&mut self) -> Box<Child<K, V>>
    where
        K: Default,
    {
        match self.child_pool.pop() {
            Some(val) => val,
            None => {
                self.n_allocs += 1;
                Box::new(Child::default())
            }
        }
    }

    pub fn alloc_reclaim(&mut self) -> Box<Reclaim<K, V>> {
        match self.reclaim_pool.pop() {
            Some(val) => val,
            None => {
                self.n_allocs += 1;
                Box::new(Reclaim::default())
            }
        }
    }

    pub fn free_node(&mut self, mut node: Box<Node<K, V>>) {
        let pool = match node.as_mut() {
            Node::Trie { bmp, childs } => {
                *bmp = 0;
                childs.clear();
                &mut self.node_trie_pool
            }
            Node::List { items } => {
                items.clear();
                &mut self.node_list_pool
            }
            Node::Tomb { .. } => &mut self.node_tomb_pool,
        };
        if pool.len() < MAX_POOL_SIZE {
            pool.push(node)
        } else {
            self.n_frees += 1
        }
    }

    pub fn free_child(&mut self, child: Box<Child<K, V>>) {
        if self.child_pool.len() < MAX_POOL_SIZE {
            self.child_pool.push(child)
        } else {
            self.n_frees += 1
        }
    }

    pub fn free_reclaim(&mut self, reclaim: Box<Reclaim<K, V>>) {
        if self.reclaim_pool.len() < MAX_POOL_SIZE {
            self.reclaim_pool.push(reclaim)
        } else {
            self.n_frees += 1
        }
    }

    pub fn swing<T>(
        &mut self,
        epoch: &Arc<AtomicU64>,
        loc: &AtomicPtr<T>,
        old: *mut T,
        new: *mut T,
    ) -> bool
    where
        V: Clone,
    {
        if loc.compare_and_swap(old, new, SeqCst) == old {
            let r = {
                let mut r = self.alloc_reclaim();
                r.epoch = Some(epoch.load(SeqCst));
                r.items.clear();
                r.items.extend(self.older.drain(..)); // TODO: can we do memcpy ?
                r
            };
            self.reclaims.push(r);
            self.newer.drain(..).for_each(|m| m.leak());
            true
        } else {
            self.older.drain(..).for_each(|om| om.leak());
            while let Some(om) = self.newer.pop() {
                match om {
                    OwnedMem::Child(val) => self.free_child(val),
                    OwnedMem::Node(val) => self.free_node(val),
                    OwnedMem::None => (),
                }
            }
            false
        }
    }

    pub fn garbage_collect(&mut self, gc_epoch: u64) {
        let n = self.reclaims.len();
        for i in (0..n).rev() {
            match self.reclaims[i].epoch {
                Some(epoch) if epoch < gc_epoch => {
                    let mut r = self.reclaims.remove(i);
                    while let Some(om) = r.items.pop() {
                        match om {
                            OwnedMem::Child(val) => self.free_child(val),
                            OwnedMem::Node(val) => self.free_node(val),
                            OwnedMem::None => (),
                        }
                    }
                    r.epoch = None;
                    self.free_reclaim(r);
                }
                Some(_) | None => (),
            }
        }
    }

    pub fn validate(&self) {
        let n = self.reclaims.len();
        debug_assert!(n < 512, "reclaims:{}", n);

        let n = self.older.len();
        debug_assert!(n < 512, "older:{}", n);

        let n = self.newer.len();
        debug_assert!(n < 512, "newer:{}", n);

        let n = self.child_pool.len();
        debug_assert!(n < 512, "child_pool:{}", n);

        let n = self.node_trie_pool.len();
        debug_assert!(n < 512, "node_trie_pool:{}", n);

        let n = self.node_list_pool.len();
        debug_assert!(n < 512, "node_list_pool:{}", n);

        let n = self.node_tomb_pool.len();
        debug_assert!(n < 512, "node_tomb_pool:{}", n);

        let n = self.reclaim_pool.len();
        debug_assert!(n < 512, "reclaim_pool:{}", n);
    }
}

pub struct Reclaim<K, V> {
    epoch: Option<u64>,
    items: Vec<OwnedMem<K, V>>,
}

impl<K, V> fmt::Debug for Reclaim<K, V> {
    fn fmt(&self, f: &mut fmt::Formatter) -> result::Result<(), fmt::Error> {
        let items = self
            .items
            .iter()
            .map(|item| match item {
                OwnedMem::Child(_) => "child",
                OwnedMem::Node(_) => "node",
                OwnedMem::None => "None",
            })
            .collect::<Vec<&str>>()
            .join("");
        write!(f, "epoch:{:?} items:{:?}", self.epoch, items)
    }
}

impl<K, V> Default for Reclaim<K, V> {
    fn default() -> Self {
        Reclaim {
            epoch: None,
            items: Vec::with_capacity(64),
        }
    }
}

pub enum Mem<K, V> {
    Child(*mut Child<K, V>),
    Node(*mut Node<K, V>),
}

enum OwnedMem<K, V> {
    Child(Box<Child<K, V>>),
    Node(Box<Node<K, V>>),
    None,
}

impl<K, V> Default for OwnedMem<K, V> {
    fn default() -> Self {
        OwnedMem::None
    }
}

impl<K, V> OwnedMem<K, V> {
    #[inline]
    fn leak(self) {
        match self {
            OwnedMem::Child(val) => {
                Box::leak(val);
            }
            OwnedMem::Node(val) => {
                Box::leak(val);
            }
            OwnedMem::None => (),
        }
    }
}
