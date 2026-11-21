use std::collections::HashMap;

pub type Person = usize;

pub struct Event {
    pub ts: i64,
    pub cents: i64,
    pub category: usize,
}
pub struct Click {
    pub id: Box<str>,
    pub ts: i64,
    pub campaign: usize,
}
#[derive(Default)]
pub struct Timeline {
    pub events: Vec<Event>,
    pub clicks: Vec<Click>,
}
pub struct UserRow {
    pub person: Person,
    pub day: i32,
    pub sessions: i64,
    pub events: i64,
    pub revenue: i64,
    pub top: Option<usize>,
}
#[derive(Default)]
pub struct CampaignStats {
    pub sessions: i64,
    pub purchases: i64,
    pub revenue: i64,
    pub assists: i64,
}
impl CampaignStats {
    pub fn add(&mut self, other: Self) {
        self.sessions += other.sessions;
        self.purchases += other.purchases;
        self.revenue += other.revenue;
        self.assists += other.assists;
    }
}
pub type CampaignDays = HashMap<(usize, i32), CampaignStats>;
pub const ORGANIC: usize = 0;
