mod csv;
mod identity;
mod input;
mod model;
mod output;
mod report;
mod time;

use input::Lookups;
use model::{CampaignDays, Click, Event, Person, Timeline};
use rayon::prelude::*;
use std::{
    collections::HashMap,
    env, fs,
    num::NonZeroUsize,
    path::{Path, PathBuf},
    time::Instant,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

fn path(key: &str, default: &str) -> PathBuf {
    env::var_os(key)
        .map(PathBuf::from)
        .unwrap_or_else(|| default.into())
}

fn output_folder(root: &Path, name: &str) -> Result<PathBuf> {
    let folder = root.join(name);
    fs::create_dir_all(&folder)?;
    if fs::read_dir(&folder)?.next().is_some() {
        return Err(format!(
            "{} must be empty; use a fresh output directory",
            folder.display()
        )
        .into());
    }
    Ok(folder)
}

fn summarize(
    group: usize,
    events: Vec<Vec<(Person, Event)>>,
    clicks: Vec<Vec<(Person, Click)>>,
    refs: &Lookups,
    folder: &Path,
) -> Result<(usize, CampaignDays)> {
    let mut people: HashMap<Person, Timeline> = HashMap::new();
    for (user, e) in events.into_iter().flatten() {
        people.entry(user).or_default().events.push(e);
    }
    for (user, c) in clicks.into_iter().flatten() {
        if let Some(person) = people.get_mut(&user) {
            person.clicks.push(c);
        }
    }
    let mut rows = Vec::new();
    let mut totals = CampaignDays::new();
    for (person, timeline) in people {
        report::shopper(person, timeline, &mut rows, &mut totals);
    }
    if !rows.is_empty() {
        output::users(
            &folder.join(format!("part-{group:03}.parquet")),
            &rows,
            refs,
        )?;
    }
    Ok((rows.len(), totals))
}

fn run() -> Result<()> {
    let start = Instant::now();
    let data = path("DATA_DIR", "/data");
    let out = path("OUTPUT_DIR", "/output");
    let workers = match env::var("NCPU") {
        Ok(n) => n.parse::<NonZeroUsize>()?,
        Err(env::VarError::NotPresent) => std::thread::available_parallelism()?,
        Err(e) => return Err(e.into()),
    }
    .get();
    rayon::ThreadPoolBuilder::new()
        .num_threads(workers)
        .build_global()?;
    let groups = workers.checked_mul(4).ok_or("Too many workers")?;
    let user_folder = output_folder(&out, "user_daily")?;
    let campaign_folder = output_folder(&out, "campaign_daily")?;
    let (people, refs) = rayon::join(
        || identity::People::read(&data.join("links")),
        || Lookups::read(&data),
    );
    let (people, refs) = (people?, refs?);
    let read_start = Instant::now();
    let (events, clicks) = rayon::join(
        || input::events(&data, &people, &refs, groups),
        || input::clicks(&data, &people, &refs, groups),
    );
    let (events, clicks) = (events?, clicks?);
    drop(people);
    let report_start = Instant::now();
    let parts: Vec<_> = events
        .into_par_iter()
        .zip(clicks)
        .enumerate()
        .map(|(group, (e, c))| summarize(group, e, c, &refs, &user_folder))
        .collect::<Result<_>>()?;
    let mut count = 0;
    let mut totals = CampaignDays::new();
    for (rows, part) in parts {
        count += rows;
        for (key, stats) in part {
            totals.entry(key).or_default().add(stats);
        }
    }
    if count == 0 {
        output::users(&user_folder.join("part-000.parquet"), &[], &refs)?;
    }
    let campaigns = totals.len();
    output::campaigns(&campaign_folder.join("part-000.parquet"), totals, &refs)?;
    eprintln!(
        "{} shopper rows, {} campaign rows; lookups {:.3}s, input and dedup {:.3}s, reports and write {:.3}s",
        count,
        campaigns,
        (read_start - start).as_secs_f64(),
        (report_start - read_start).as_secs_f64(),
        report_start.elapsed().as_secs_f64()
    );
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("shopping-sessions: {e}");
        std::process::exit(1);
    }
}
