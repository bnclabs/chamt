use arbitrary::{self, unstructured::Unstructured, Arbitrary};
use rand::{prelude::random, rngs::SmallRng, Rng, SeedableRng};

use std::{collections::BTreeMap, mem, thread};

use super::*;

#[test]
fn test_list_operation() {
    let mut items: Vec<Item<u64>> = vec![
        Item {
            key: 20,
            value: 200,
        },
        Item {
            key: 10,
            value: 100,
        },
        Item {
            key: 50,
            value: 500,
        },
        Item {
            key: 30,
            value: 300,
        },
    ];

    assert_eq!(update_into_list(10, &1000, &mut items), Some(100));
    assert_eq!(update_into_list(10, &10000, &mut items), Some(1000));
    assert_eq!(update_into_list(60, &600, &mut items), None);

    let (items, item) = remove_from_list(20, &items).unwrap();
    assert_eq!(item, 200);
    let (items, item) = remove_from_list(60, &items).unwrap();
    assert_eq!(item, 600);
    assert_eq!(remove_from_list(20, &items), None);
    assert_eq!(remove_from_list(60, &items), None);

    assert_eq!(get_from_list(10, &items), Some(10000));
    assert_eq!(get_from_list(50, &items), Some(500));
    assert_eq!(get_from_list(30, &items), Some(300));
    assert_eq!(get_from_list(20, &items), None);

    assert_eq!(
        items,
        vec![
            Item {
                key: 10,
                value: 10000,
            },
            Item {
                key: 50,
                value: 500,
            },
            Item {
                key: 30,
                value: 300,
            },
        ]
    );
}

#[test]
fn test_hamming_distance() {
    let bmp = 0xaaaa;
    for w in 0..=255 {
        let o = ((w % 128) / 2) as usize;
        let dist = hamming_distance(w, bmp.clone());
        match w % 2 {
            0 if w < 128 => assert_eq!(dist, Distance::Insert(o)),
            0 => assert_eq!(dist, Distance::Insert(64 + o)),
            1 if w < 128 => assert_eq!(dist, Distance::Set(o)),
            1 => assert_eq!(dist, Distance::Set(64 + o)),
            _ => unreachable!(),
        }
    }

    let bmp = 0x5555;
    for w in 0..=255 {
        let o = ((w % 128) / 2) as usize;
        let dist = hamming_distance(w, bmp.clone());
        match w % 2 {
            0 if w < 128 => assert_eq!(dist, Distance::Set(o)),
            0 => assert_eq!(dist, Distance::Set(64 + o)),
            1 if w < 128 => assert_eq!(dist, Distance::Insert(o + 1)),
            1 => assert_eq!(dist, Distance::Insert(64 + o + 1)),
            _ => unreachable!(),
        }
    }
}

#[test]
fn test_map() {
    let seed: u128 = random();
    let seed: u128 = 108608880608704922882102056739567863183;
    println!("test_map seed {}", seed);

    let n_ops = 2_000_000; // TODO
    let n_threads = 8; // TODO
    let modul = u32::MAX / n_threads;
    // let modul = 65536 / n_threads;

    let map: Map<u64> = Map::new();
    let mut handles = vec![];
    for id in 0..n_threads {
        let seed = seed + ((id as u128) * 100);

        let map = map.cloned();
        let btmap: BTreeMap<u32, u64> = BTreeMap::new();
        let h = thread::spawn(move || with_btreemap(id, seed, modul, n_ops, map, btmap));

        handles.push(h);
    }

    let mut btmap = BTreeMap::new();
    for handle in handles.into_iter() {
        btmap = merge_btmap([btmap, handle.join().unwrap()]);
    }

    println!("len {}", map.len());
    assert_eq!(map.len(), btmap.len());

    for (key, val) in btmap.iter() {
        assert_eq!(map.get(*key), Some(val.clone()));
    }

    mem::drop(map);
    mem::drop(btmap);
}

fn with_btreemap(
    id: u32,
    seed: u128,
    modul: u32,
    n_ops: usize,
    map: Map<u64>,
    mut btmap: BTreeMap<u32, u64>,
) -> BTreeMap<u32, u64> {
    let mut rng = SmallRng::from_seed(seed.to_le_bytes());

    let mut counts = [[0_usize; 2]; 3];

    for _i in 0..n_ops {
        let bytes = rng.gen::<[u8; 32]>();
        let mut uns = Unstructured::new(&bytes);

        let mut op: Op<u64> = uns.arbitrary().unwrap();
        op = op.adjust_key(id, modul);
        // println!("{}-op -- {:?}", id, op);
        match op.clone() {
            Op::Set(key, value) => {
                // map.print();

                let map_val = map.set(key, value).unwrap();
                let btmap_val = btmap.insert(key, value);
                if map_val != btmap_val {
                    map.print();
                }
                counts[0][0] += 1;
                counts[0][1] += if map_val.is_none() { 0 } else { 1 };

                assert_eq!(map_val, btmap_val, "key {}", key);
            }
            Op::Remove(key) => {
                // map.print();

                let map_val = map.remove(key);
                let btmap_val = btmap.remove(&key);
                if map_val != btmap_val {
                    map.print();
                }

                counts[1][0] += 1;
                counts[1][1] += if map_val.is_none() { 0 } else { 1 };

                assert_eq!(map_val, btmap_val, "key {}", key);
            }
            Op::Get(key) => {
                // map.print();

                let map_val = map.get(key);
                let btmap_val = btmap.get(&key).cloned();
                if map_val != btmap_val {
                    map.print();
                }

                counts[2][0] += 1;
                counts[2][1] += if map_val.is_none() { 0 } else { 1 };

                assert_eq!(map_val, btmap_val, "key {}", key);
            }
        };
    }

    println!("{} counts {:?}", id, counts);
    btmap
}

#[derive(Clone, Debug, Arbitrary)]
enum Op<V> {
    Get(u32),
    Set(u32, V),
    Remove(u32),
}

impl<V> Op<V> {
    fn adjust_key(self, id: u32, modul: u32) -> Self {
        match self {
            Op::Get(key) => {
                let key = (id * modul) + (key % modul);
                Op::Get((id * modul) + (key % modul))
            }
            Op::Set(key, value) => {
                let key = (id * modul) + (key % modul);
                Op::Set(key, value)
            }
            Op::Remove(key) => {
                let key = (id * modul) + (key % modul);
                Op::Remove(key)
            }
        }
    }
}

fn merge_btmap(items: [BTreeMap<u32, u64>; 2]) -> BTreeMap<u32, u64> {
    let [mut one, two] = items;

    for (key, value) in two.iter() {
        one.insert(*key, *value);
    }
    one
}
