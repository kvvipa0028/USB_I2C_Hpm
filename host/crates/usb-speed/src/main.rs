//! usb-speed: host-side throughput/stall measurement for the usb_speed
//! test firmware (VID:PID 1209:0001, bulk OUT 0x01, bulk IN 0x81).

mod verify;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use nusb::transfer::{Buffer, Bulk, In, Out};
use nusb::MaybeFuture;
use verify::Verifier;

const VID: u16 = 0x1209;
const PID: u16 = 0x0001;
const EP_IN: u8 = 0x81;
const EP_OUT: u8 = 0x01;

#[derive(Parser)]
#[command(name = "usb-speed", about = "USB HS bulk throughput test")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List devices matching 1209:0001
    List,
    /// Device -> host (bulk IN) throughput + stall measurement
    Rx {
        #[arg(long, default_value_t = 60)]
        seconds: u64,
        #[arg(long, default_value_t = 1024 * 1024)]
        transfer_size: usize,
        #[arg(long, default_value_t = 8)]
        queue: usize,
        #[arg(long)]
        csv: Option<PathBuf>,
    },
    /// Host -> device (bulk OUT) throughput + stall measurement
    Tx {
        #[arg(long, default_value_t = 60)]
        seconds: u64,
        #[arg(long, default_value_t = 1024 * 1024)]
        transfer_size: usize,
        #[arg(long, default_value_t = 8)]
        queue: usize,
    },
}

#[derive(Default)]
struct Stats {
    bytes: u64,
    per_sec: Vec<u64>, // bytes completed per whole second
    longest_gap: Duration,
    errors: u64,
    last_completion: Option<Instant>,
}

impl Stats {
    fn record(&mut self, nbytes: usize, now: Instant, start: Instant) {
        self.bytes += nbytes as u64;
        let sec = now.duration_since(start).as_secs() as usize;
        if self.per_sec.len() <= sec {
            self.per_sec.resize(sec + 1, 0);
        }
        self.per_sec[sec] += nbytes as u64;
        if let Some(prev) = self.last_completion {
            let gap = now.duration_since(prev);
            if gap > self.longest_gap {
                self.longest_gap = gap;
            }
        }
        self.last_completion = Some(now);
    }
}

fn mib(b: u64) -> f64 {
    b as f64 / (1024.0 * 1024.0)
}

fn open_interface() -> anyhow::Result<nusb::Interface> {
    let info = nusb::list_devices()
        .wait()?
        .find(|d| d.vendor_id() == VID && d.product_id() == PID)
        .ok_or_else(|| anyhow::anyhow!("no device {:04x}:{:04x} found", VID, PID))?;
    let dev = info.open().wait()?;
    Ok(dev.claim_interface(0).wait()?)
}

fn cmd_list() -> anyhow::Result<()> {
    let mut found = false;
    for d in nusb::list_devices().wait()? {
        if d.vendor_id() == VID && d.product_id() == PID {
            found = true;
            println!(
                "{:04x}:{:04x} bus={} addr={} serial={} speed={:?}",
                d.vendor_id(),
                d.product_id(),
                d.bus_id(),
                d.device_address(),
                d.serial_number().unwrap_or("?"),
                d.speed()
            );
        }
    }
    if !found {
        println!("no {:04x}:{:04x} device found", VID, PID);
    }
    Ok(())
}

fn print_summary(s: &Stats, elapsed: Duration, v: Option<&Verifier>) {
    println!("--- summary ---");
    println!(
        "total: {:.1} MiB in {:.2?} = {:.1} MiB/s",
        mib(s.bytes),
        elapsed,
        mib(s.bytes) / elapsed.as_secs_f64()
    );
    if !s.per_sec.is_empty() {
        let min = s.per_sec.iter().copied().min().unwrap_or(0);
        let max = s.per_sec.iter().copied().max().unwrap_or(0);
        println!(
            "per-second MiB/s: min={:.1} max={:.1}",
            mib(min),
            mib(max)
        );
    }
    println!(
        "longest completion gap (host user-space): {:.2} ms",
        s.longest_gap.as_secs_f64() * 1000.0
    );
    println!("transfer errors: {}", s.errors);
    if let Some(v) = v {
        println!("words verified: {}", v.words());
        println!("discontinuities: {}", v.count);
        for d in &v.samples {
            println!(
                "  word_offset={} expected={} got={}",
                d.word_offset, d.expected, d.got
            );
        }
    }
}

fn report_seconds(stats: &Stats, last: &mut u64, now: Instant, start: Instant) {
    let sec = now.duration_since(start).as_secs();
    if sec > *last {
        for s in *last..sec {
            let b = stats.per_sec.get(s as usize).copied().unwrap_or(0);
            println!("t+{:>3}s: {:.1} MiB/s", s + 1, mib(b));
        }
        *last = sec;
    }
}

fn cmd_rx(
    seconds: u64,
    transfer_size: usize,
    queue: usize,
    csv: Option<PathBuf>,
) -> anyhow::Result<()> {
    let iface = open_interface()?;
    let mut ep = iface.endpoint::<Bulk, In>(EP_IN)?;

    for _ in 0..queue {
        ep.submit(ep.allocate(transfer_size));
    }

    let start = Instant::now();
    let deadline = Duration::from_secs(seconds);
    let mut stats = Stats::default();
    let mut ver = Verifier::new();
    let mut last_sec = 0u64;

    while start.elapsed() < deadline {
        let Some(c) = ep.wait_next_complete(Duration::from_millis(200)) else {
            continue;
        };
        let now = Instant::now();
        match c.status {
            Ok(()) => {
                stats.record(c.actual_len, now, start);
                ver.check_bytes(&c.buffer[..c.actual_len]);
            }
            Err(_) => stats.errors += 1,
        }
        if start.elapsed() < deadline {
            ep.submit(c.buffer);
        }
        report_seconds(&stats, &mut last_sec, now, start);
    }

    drop(ep);
    let elapsed = start.elapsed();

    if let Some(path) = csv {
        let mut w = String::from("second,mib_per_sec\n");
        for (i, b) in stats.per_sec.iter().enumerate() {
            w.push_str(&format!("{},{:.2}\n", i + 1, mib(*b)));
        }
        std::fs::write(&path, w)?;
        println!("wrote {}", path.display());
    }

    print_summary(&stats, elapsed, Some(&ver));
    Ok(())
}

fn cmd_tx(seconds: u64, transfer_size: usize, queue: usize) -> anyhow::Result<()> {
    let iface = open_interface()?;
    let mut ep = iface.endpoint::<Bulk, Out>(EP_OUT)?;

    for _ in 0..queue {
        let mut buf = vec![0u8; transfer_size];
        for (i, c) in buf.chunks_exact_mut(4).enumerate() {
            c.copy_from_slice(&(i as u32).to_le_bytes());
        }
        ep.submit(Buffer::from(buf));
    }

    let start = Instant::now();
    let deadline = Duration::from_secs(seconds);
    let mut stats = Stats::default();
    let mut last_sec = 0u64;

    while start.elapsed() < deadline {
        let Some(c) = ep.wait_next_complete(Duration::from_millis(200)) else {
            continue;
        };
        let now = Instant::now();
        match c.status {
            Ok(()) => stats.record(c.actual_len, now, start),
            Err(_) => stats.errors += 1,
        }
        if start.elapsed() < deadline {
            ep.submit(c.buffer);
        }
        report_seconds(&stats, &mut last_sec, now, start);
    }

    drop(ep);
    print_summary(&stats, start.elapsed(), None);
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::List => cmd_list(),
        Cmd::Rx {
            seconds,
            transfer_size,
            queue,
            csv,
        } => cmd_rx(seconds, transfer_size, queue, csv),
        Cmd::Tx {
            seconds,
            transfer_size,
            queue,
        } => cmd_tx(seconds, transfer_size, queue),
    }
}
