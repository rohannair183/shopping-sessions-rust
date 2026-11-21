"""Add two stage timers and isolated-reader entry points to a benchmark copy."""
from pathlib import Path
from shutil import copytree
import sys

root = Path(__file__).resolve().parent
variant = sys.argv[1]
dest = root / ("probe-" + variant)
copytree(root / variant, dest, dirs_exist_ok=True)

def replace(file, needle, replacement):
    text = file.read_text()
    if text.count(needle) != 1:
        raise ValueError(f"{file}: expected one {needle!r}")
    file.write_text(text.replace(needle, replacement))

main = dest / "src/main.rs"
inp = dest / "src/input.rs"
needle = "    let read_start = Instant::now();"
replace(main, needle, '''    if let Ok(which) = env::var("READER_ONLY") {
        match which.as_str() {
            "event" => { input::events(&data, &people, &refs, groups)?; }
            "click" => { input::clicks(&data, &people, &refs, groups)?; }
            _ => return Err("Unknown READER_ONLY".into()),
        }
        return Ok(());
    }
''' + needle)
needle = "|| input::events(&data, &people, &refs, groups),"
replace(main, needle, '''|| {
            let at = Instant::now();
            let result = input::events(&data, &people, &refs, groups);
            eprintln!("event_scan_and_route_s={:.6}", at.elapsed().as_secs_f64());
            result
        },''')

event_file = inp
if variant in ("grouped","combined"):
    event_file = dest / "src/input/grouped.rs"
    replace(event_file, "    out\n}", '''    if std::env::var_os("AUDIT_EVENT_ALIASES").is_some() {
        eprintln!("raw_resolves={}", inline.len() + text.len());
    }
    out
}''')
    event_start = "    let scanned = event_read(data, refs, count.next_power_of_two())?;"
    event_end = event_start
    event_data = "scanned"
    count_events = "scanned.iter().flat_map(|part| &part.versions).map(|v| v.inline.len() + v.text.len()).sum::<usize>()"
else:
    event_start = '    let batches: Batches<Candidate> = chunks(&data.join("events"))?'
    event_end = "    let winners = regroup(batches, count)"
    event_data = "batches"
    count_events = "batches.iter().flatten().map(Vec::len).sum::<usize>()"

replace(event_file, event_start, "    let stage_start = std::time::Instant::now();\n" + event_start)
event_timer = f'''    eprintln!("event_read_s={{:.6}}", stage_start.elapsed().as_secs_f64());
    if std::env::var("READER_ONLY").as_deref() == Ok("event") {{
        std::hint::black_box(&{event_data});
        eprintln!("records={{}}", {count_events});
        return Ok(Vec::new());
    }}
'''
replace(event_file, event_end, event_end + "\n" + event_timer if variant in ("grouped","combined") else event_timer + event_end)
click_start = '    let batches = chunks(&data.join("clicks"))?'
replace(inp, click_start, "    let stage_start = std::time::Instant::now();\n" +
        click_start.replace("let batches =", "let batches: Batches<OwnedClick> ="))
click_end = "    Ok(regroup(batches, count))"
replace(inp, click_end, '''    eprintln!("click_read_s={:.6}", stage_start.elapsed().as_secs_f64());
    if std::env::var("READER_ONLY").as_deref() == Ok("click") {
        std::hint::black_box(&batches);
        eprintln!("records={}", batches.iter().flatten().map(Vec::len).sum::<usize>());
        return Ok(Vec::new());
    }
''' + click_end)
