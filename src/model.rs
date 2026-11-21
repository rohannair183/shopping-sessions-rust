pub type Person = usize;

pub struct Event {
    pub ts: i64,
    pub cents: i64,
    pub category: usize,
}

// Generated click IDs use at most nine bytes: a prefix and eight base-36 digits.
const INLINE_CLICK_BYTES: usize = 9;

pub enum ClickId {
    Inline {
        bytes: [u8; INLINE_CLICK_BYTES],
        len: u8,
    },
    Text(Box<str>),
}

impl From<&str> for ClickId {
    fn from(id: &str) -> Self {
        if id.len() > INLINE_CLICK_BYTES {
            return Self::Text(id.into());
        }
        let mut bytes = [0; INLINE_CLICK_BYTES];
        bytes[..id.len()].copy_from_slice(id.as_bytes());
        Self::Inline {
            bytes,
            len: id.len() as u8,
        }
    }
}

impl ClickId {
    pub fn bytes(&self) -> &[u8] {
        match self {
            Self::Inline { bytes, len } => &bytes[..usize::from(*len)],
            Self::Text(id) => id.as_bytes(),
        }
    }
}

pub struct Click {
    pub id: ClickId,
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
#[derive(Clone, Default)]
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
pub const ORGANIC: usize = 0;
