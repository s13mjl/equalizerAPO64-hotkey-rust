//! Samples private commit, working set and GDI/USER object counts of a process.

use std::mem::size_of;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    GetGuiResources, OpenProcess, GR_GDIOBJECTS, GR_USEROBJECTS,
    PROCESS_QUERY_INFORMATION, PROCESS_VM_READ,
};

#[derive(Default, Clone, Copy)]
struct Sample {
    private_mb: f64,
    working_set_mb: f64,
    pagefile_mb: f64,
    gdi: u32,
    user: u32,
}

impl Sample {
    fn measure(process: HANDLE) -> Result<Self, String> {
        let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
        let ok = unsafe {
            K32GetProcessMemoryInfo(
                process,
                &mut counters as *mut PROCESS_MEMORY_COUNTERS_EX as *mut PROCESS_MEMORY_COUNTERS,
                size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            )
        };
        if !ok.as_bool() {
            return Err("K32GetProcessMemoryInfo failed".into());
        }
        let gdi = unsafe { GetGuiResources(process, GR_GDIOBJECTS) };
        let user = unsafe { GetGuiResources(process, GR_USEROBJECTS) };
        Ok(Self {
            private_mb: counters.PrivateUsage as f64 / 1_048_576.0,
            working_set_mb: counters.WorkingSetSize as f64 / 1_048_576.0,
            pagefile_mb: counters.PagefileUsage as f64 / 1_048_576.0,
            gdi,
            user,
        })
    }
}

fn usage() -> ! {
    eprintln!(
        "usage: mem-probe (--pid <pid> | --name <exe>) [--samples N] [--interval-ms M]"
    );
    std::process::exit(2);
}

fn arg_value(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn find_pid_by_name(name: &str) -> Result<u32, String> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
        .map_err(|error| format!("CreateToolhelp32Snapshot failed: {error}"))?;
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut found = None;
    if unsafe { Process32FirstW(snapshot, &mut entry) }.is_ok() {
        loop {
            let end = entry
                .szExeFile
                .iter()
                .position(|c| *c == 0)
                .unwrap_or(entry.szExeFile.len());
            let exe = String::from_utf16_lossy(&entry.szExeFile[..end]);
            if exe.eq_ignore_ascii_case(name) {
                found = Some(entry.th32ProcessID);
                break;
            }
            if unsafe { Process32NextW(snapshot, &mut entry) }.is_err() {
                break;
            }
        }
    }
    let _ = unsafe { CloseHandle(snapshot) };
    found.ok_or_else(|| format!("process \"{name}\" not found"))
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if values.is_empty() {
        return 0.0;
    }
    values[values.len() / 2]
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        usage();
    }
    let pid = match (arg_value(&args, "--pid"), arg_value(&args, "--name")) {
        (Some(pid), _) => pid.parse::<u32>().unwrap_or_else(|_| usage()),
        (None, Some(name)) => find_pid_by_name(&name).unwrap_or_else(|error| {
            eprintln!("{error}");
            std::process::exit(1);
        }),
        _ => usage(),
    };
    let samples = arg_value(&args, "--samples")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);
    let interval = arg_value(&args, "--interval-ms")
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(500);

    let process = unsafe {
        OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_VM_READ, false, pid)
    }
    .unwrap_or_else(|error| {
        eprintln!("OpenProcess({pid}) failed: {error}");
        std::process::exit(1);
    });

    let mut collected = Vec::with_capacity(samples);
    let started = Instant::now();
    for index in 0..samples {
        let sample = Sample::measure(process).unwrap_or_else(|error| {
            eprintln!("{error}");
            std::process::exit(1);
        });
        collected.push(sample);
        println!(
            "{:>3}  t={:>7.2}s  private={:>7.2} MB  ws={:>7.2} MB  pagefile={:>7.2} MB  gdi={}  user={}",
            index + 1,
            started.elapsed().as_secs_f64(),
            sample.private_mb,
            sample.working_set_mb,
            sample.pagefile_mb,
            sample.gdi,
            sample.user
        );
        if index + 1 < samples {
            std::thread::sleep(Duration::from_millis(interval));
        }
    }
    let _ = unsafe { CloseHandle(process) };

    let mut private = collected.iter().map(|sample| sample.private_mb).collect::<Vec<_>>();
    let mut working = collected.iter().map(|sample| sample.working_set_mb).collect::<Vec<_>>();
    println!(
        "median: private={:.2} MB  ws={:.2} MB  min_private={:.2}  max_private={:.2}",
        median(&mut private),
        median(&mut working),
        collected.iter().map(|s| s.private_mb).fold(f64::INFINITY, f64::min),
        collected.iter().map(|s| s.private_mb).fold(0.0, f64::max)
    );
}
