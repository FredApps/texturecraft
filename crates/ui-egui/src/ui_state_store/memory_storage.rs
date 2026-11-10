/// eframe's storage held in memory
#[derive(Default)]
pub(crate) struct MemoryStorage {
    pub(crate) values: std::collections::HashMap<String, String>,
    pub(crate) rejected_key: Option<&'static str>,
}

impl eframe::Storage for MemoryStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        if self.rejected_key != Some(key) {
            self.values.insert(key.to_owned(), value);
        }
    }

    fn remove_string(&mut self, key: &str) {
        self.values.remove(key);
    }

    fn flush(&mut self) {}
}
