mod campaign;
mod csv;
mod identity;
mod input;
mod model;
mod output;
mod report;
mod time;

use campaign::CampaignDays;
use identity::Names;
use input::Lookups;
use model::{Click, Event, Person, Timeline};
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
    names: &Names,
    folder: &Path,
    groups: usize,
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
    let mut totals = refs.campaign_totals(groups);
    for (person, timeline) in people {
        report::shopper(person, timeline, &mut rows, &mut totals);
    }
    if !rows.is_empty() {
        output::users(
            &folder.join(format!("part-{group:03}.parquet")),
            &rows,
            refs,
            names,
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
    let refs = Lookups::read(&data)?;
    let read_start = Instant::now();
    let (links_and_clicks, winners) = rayon::join(
        || -> Result<_> {
            let people = identity::People::read(&data.join("links"))?;
            let clicks = input::clicks(&data, &people, &refs, groups)?;
            Ok((people, clicks))
        },
        || input::read_winners(&data, &refs, groups),
    );
    let (people, clicks) = links_and_clicks?;
    let events = input::route_winners(winners?, groups, &refs, &people);
    let names = people.into_names();
    let report_start = Instant::now();
    let parts: Vec<_> = events
        .into_par_iter()
        .zip(clicks)
        .enumerate()
        .map(|(group, (e, c))| summarize(group, e, c, &refs, &names, &user_folder, groups))
        .collect::<Result<_>>()?;
    let mut count = 0;
    let mut totals = refs.campaign_totals(groups);
    for (rows, part) in parts {
        count += rows;
        totals.merge(part);
    }
    if count == 0 {
        output::users(&user_folder.join("part-000.parquet"), &[], &refs, &names)?;
    }
    let totals = totals.into_rows();
    let campaigns = totals.len();
    output::campaigns(&campaign_folder.join("part-000.parquet"), &totals, &refs)?;
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
