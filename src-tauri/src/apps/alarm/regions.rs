//! The service's regions as a tree: oblast-level regions (Kyiv and Crimea among them), their
//! districts and the districts' communities. The table is built by `scripts/alarm-regions.mjs`
//! from the service's own region geometry and compiled in. Names are the service's (Ukrainian).

use std::{collections::HashMap, sync::OnceLock};

const TABLE: &str = include_str!("regions.tsv");

#[derive(Debug, PartialEq, Eq)]
pub struct Region {
    pub id: u32,
    /// 0 — an oblast-level region.
    pub parent: u32,
    pub name: &'static str,
}

struct Tree {
    list: Vec<Region>,
    by_id: HashMap<u32, usize>,
}

fn tree() -> &'static Tree {
    static TREE: OnceLock<Tree> = OnceLock::new();
    TREE.get_or_init(|| {
        let list: Vec<Region> = TABLE
            .lines()
            .filter_map(|line| {
                let mut cells = line.split('\t');
                Some(Region {
                    id: cells.next()?.parse().ok()?,
                    parent: cells.next()?.parse().ok()?,
                    name: cells.next()?,
                })
            })
            .collect();
        let by_id = list.iter().enumerate().map(|(i, r)| (r.id, i)).collect();
        Tree { list, by_id }
    })
}

pub fn get(id: u32) -> Option<&'static Region> {
    let t = tree();
    t.by_id.get(&id).map(|&i| &t.list[i])
}

/// The region and the ones it is in, from itself up to its oblast.
pub fn chain(id: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut at = get(id);
    while let Some(region) = at {
        if out.contains(&region.id) {
            break;
        }
        out.push(region.id);
        at = get(region.parent);
    }
    out
}

/// "Броварський район, Київська область": where it is, without itself.
pub fn place_of(id: u32) -> String {
    chain(id)
        .into_iter()
        .skip(1)
        .filter_map(get)
        .map(|r| r.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// The apostrophe is typed in three ways; case doesn't matter.
fn fold(text: &str) -> String {
    text.to_lowercase().replace(['\'', 'ʼ', '`'], "’")
}

/// Regions whose name has the typed text: a whole word first, then a word's start, then any
/// part; within each the wider first (oblasts, then districts, then communities), then
/// alphabetically. Empty text — the oblast-level ones.
pub fn search(query: &str, limit: usize) -> Vec<&'static Region> {
    let query = fold(query.trim());
    let depth = |r: &Region| chain(r.id).len();
    let fit = |r: &Region| {
        let name = fold(r.name);
        let mut words = name.split(|c: char| !c.is_alphanumeric() && c != '’' && c != '-');
        if words.clone().any(|w| w == query) {
            0
        } else if words.any(|w| w.starts_with(&query)) {
            1
        } else {
            2
        }
    };
    let mut found: Vec<&Region> = tree()
        .list
        .iter()
        .filter(|r| {
            if query.is_empty() {
                r.parent == 0
            } else {
                fold(r.name).contains(&query)
            }
        })
        .collect();
    found.sort_by(|a, b| {
        fit(a)
            .cmp(&fit(b))
            .then_with(|| depth(a).cmp(&depth(b)))
            .then_with(|| fold(a.name).cmp(&fold(b.name)))
    });
    found.truncate(limit);
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_region_reaches_an_oblast() {
        let t = tree();
        assert!(t.list.len() > 1500, "{}", t.list.len());
        assert_eq!(t.list.iter().filter(|r| r.parent == 0).count(), 26);
        for r in &t.list {
            let chain = chain(r.id);
            assert!((1..=3).contains(&chain.len()), "{}", r.name);
            assert_eq!(get(*chain.last().unwrap()).unwrap().parent, 0, "{}", r.name);
        }
    }

    #[test]
    fn a_community_is_in_its_district_and_oblast() {
        // Vinnytsia city's community, Vinnytskyi district, Vinnytska oblast.
        assert_eq!(chain(155), [155, 36, 4]);
        assert_eq!(place_of(155), "Вінницький район, Вінницька область");
        assert_eq!(chain(31), [31], "Kyiv is oblast-level");
        assert!(chain(123_456).is_empty());
    }

    #[test]
    fn search_finds_by_part_of_the_name_wider_first() {
        let names: Vec<&str> = search("бровар", 10).iter().map(|r| r.name).collect();
        assert_eq!(names[0], "Броварський район");
        assert!(names.contains(&"м. Бровари та Броварська територіальна громада"));
        assert_eq!(search("", 100).len(), 26);
        assert_eq!(search("КИЇВ", 1)[0].name, "м. Київ");
    }

    #[test]
    fn the_apostrophe_may_be_typed_any_way() {
        let typed = search("кам'янськ", 50);
        assert!(!typed.is_empty());
        assert_eq!(typed, search("кам’янськ", 50));
    }
}
