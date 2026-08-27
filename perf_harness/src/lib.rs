use std::{
    fs::File,
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, get_current_pid};
use thiserror::Error;

pub const DEFAULT_BUFFER_SIZE: usize = 1024 * 1024;
const MAGIC: &[u8; 8] = b"LANPERF1";
const SUCCESS_ACK: u8 = 1;

#[derive(Debug, Error)]
pub enum PerfError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("peer sent an invalid performance protocol header")]
    InvalidHeader,
    #[error("peer closed after {received} of {expected} bytes")]
    UnexpectedEof { received: u64, expected: u64 },
    #[error("receiver did not confirm durable completion")]
    MissingCompletionAck,
    #[error("buffer size must be between 64 KiB and 4 MiB")]
    InvalidBufferSize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TransferMetrics {
    pub bytes: u64,
    pub elapsed_seconds: f64,
    pub data_copy_seconds: f64,
    pub final_sync_seconds: Option<f64>,
    pub megabytes_per_second: f64,
    pub peak_memory_bytes: u64,
    pub average_cpu_percent: f32,
}

#[derive(Debug, Default)]
struct ProcessMetrics {
    peak_memory_bytes: u64,
    average_cpu_percent: f32,
}

struct ProcessMonitor {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<ProcessMetrics>>,
}

impl ProcessMonitor {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let handle = thread::spawn(move || monitor_process(thread_stop));
        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn finish(mut self) -> ProcessMetrics {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .and_then(|handle| handle.join().ok())
            .unwrap_or_default()
    }
}

fn monitor_process(stop: Arc<AtomicBool>) -> ProcessMetrics {
    let pid = get_current_pid().unwrap_or(Pid::from_u32(std::process::id()));
    let refresh_kind = ProcessRefreshKind::nothing().with_memory().with_cpu();
    let mut system = System::new();
    let mut peak_memory_bytes = 0;
    let mut cpu_total = 0.0_f32;
    let mut cpu_samples = 0_u32;

    while !stop.load(Ordering::Relaxed) {
        system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, refresh_kind);
        if let Some(process) = system.process(pid) {
            peak_memory_bytes = peak_memory_bytes.max(process.memory());
            cpu_total += process.cpu_usage();
            cpu_samples += 1;
        }
        thread::sleep(Duration::from_millis(250));
    }

    ProcessMetrics {
        peak_memory_bytes,
        average_cpu_percent: if cpu_samples == 0 {
            0.0
        } else {
            cpu_total / cpu_samples as f32
        },
    }
}

pub fn send_file(
    connect: SocketAddr,
    file_path: &Path,
    buffer_size: usize,
) -> Result<TransferMetrics, PerfError> {
    validate_buffer_size(buffer_size)?;
    let file = File::open(file_path)?;
    let file_size = file.metadata()?.len();
    let mut stream = TcpStream::connect(connect)?;
    configure_stream(&stream)?;

    stream.write_all(MAGIC)?;
    stream.write_all(&file_size.to_be_bytes())?;

    let monitor = ProcessMonitor::start();
    let started = Instant::now();
    let mut reader = file;
    let mut buffer = vec![0_u8; buffer_size];
    let mut sent = 0_u64;
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        stream.write_all(&buffer[..count])?;
        sent += count as u64;
    }
    stream.flush()?;
    stream.shutdown(Shutdown::Write)?;
    let data_copy_elapsed = started.elapsed();

    let mut ack = [0_u8; 1];
    stream.read_exact(&mut ack)?;
    if ack[0] != SUCCESS_ACK {
        return Err(PerfError::MissingCompletionAck);
    }
    let elapsed = started.elapsed();
    let process = monitor.finish();
    Ok(metrics(sent, elapsed, data_copy_elapsed, None, process))
}

pub fn receive_file(
    listen: SocketAddr,
    output_path: &Path,
    buffer_size: usize,
) -> Result<TransferMetrics, PerfError> {
    let listener = TcpListener::bind(listen)?;
    receive_once(listener, output_path, buffer_size)
}

pub fn receive_once(
    listener: TcpListener,
    output_path: &Path,
    buffer_size: usize,
) -> Result<TransferMetrics, PerfError> {
    validate_buffer_size(buffer_size)?;
    let (mut stream, _) = listener.accept()?;
    configure_stream(&stream)?;

    let mut magic = [0_u8; 8];
    stream.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(PerfError::InvalidHeader);
    }
    let mut size_bytes = [0_u8; 8];
    stream.read_exact(&mut size_bytes)?;
    let expected = u64::from_be_bytes(size_bytes);

    let mut writer = File::create(output_path)?;
    let mut buffer = vec![0_u8; buffer_size];
    let mut received = 0_u64;
    let monitor = ProcessMonitor::start();
    let started = Instant::now();
    while received < expected {
        let remaining = (expected - received).min(buffer_size as u64) as usize;
        let count = stream.read(&mut buffer[..remaining])?;
        if count == 0 {
            return Err(PerfError::UnexpectedEof { received, expected });
        }
        writer.write_all(&buffer[..count])?;
        received += count as u64;
    }
    writer.flush()?;
    let data_copy_elapsed = started.elapsed();
    let sync_started = Instant::now();
    writer.sync_all()?;
    let final_sync_elapsed = sync_started.elapsed();
    stream.write_all(&[SUCCESS_ACK])?;
    stream.flush()?;
    let elapsed = started.elapsed();
    let process = monitor.finish();
    Ok(metrics(
        received,
        elapsed,
        data_copy_elapsed,
        Some(final_sync_elapsed),
        process,
    ))
}

fn configure_stream(stream: &TcpStream) -> io::Result<()> {
    stream.set_nodelay(false)
}

fn validate_buffer_size(buffer_size: usize) -> Result<(), PerfError> {
    if !(64 * 1024..=4 * 1024 * 1024).contains(&buffer_size) {
        return Err(PerfError::InvalidBufferSize);
    }
    Ok(())
}

fn metrics(
    bytes: u64,
    elapsed: Duration,
    data_copy_elapsed: Duration,
    final_sync_elapsed: Option<Duration>,
    process: ProcessMetrics,
) -> TransferMetrics {
    let elapsed_seconds = elapsed.as_secs_f64().max(f64::EPSILON);
    TransferMetrics {
        bytes,
        elapsed_seconds,
        data_copy_seconds: data_copy_elapsed.as_secs_f64(),
        final_sync_seconds: final_sync_elapsed.map(|duration| duration.as_secs_f64()),
        megabytes_per_second: bytes as f64 / 1_000_000.0 / elapsed_seconds,
        peak_memory_bytes: process.peak_memory_bytes,
        average_cpu_percent: process.average_cpu_percent,
    }
}
