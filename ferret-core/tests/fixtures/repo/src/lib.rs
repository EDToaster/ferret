pub mod utils;

pub trait Searchable {
    fn search(&self, query: &str) -> Vec<String>;
}

pub fn search_all(items: &[&dyn Searchable], query: &str) -> Vec<String> {
    items.iter().flat_map(|item| item.search(query)).collect()
}

pub struct Index {
    entries: Vec<String>,
}

impl Searchable for Index {
    fn search(&self, query: &str) -> Vec<String> {
        self.entries
            .iter()
            .filter(|e| e.contains(query))
            .cloned()
            .collect()
    }
}
