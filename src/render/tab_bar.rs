use crate::tabs::TabId;

#[derive(Clone, Debug)]
pub struct TabBarEntry {
    pub id: TabId,
    pub title: String,
    pub active: bool,
}
