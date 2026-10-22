//! Invoke one local classifier through the existing MAS lifetime guardian.
use super::config::Config;
use mas_host_privacy::{Limits, NeuralAssessment, Source};
use mas_host_worker::Worker;
use std::{
    collections::BTreeMap,
    fs::File,
    os::fd::{AsRawFd, FromRawFd},
};

pub(super) fn inspect(
    config: &Config,
    source: &Source,
    limits: Limits,
) -> Result<NeuralAssessment, &'static str> {
    let request = config
        .pins
        .request(source, limits)
        .map_err(|_| "request-limit")?;
    let encoded = serde_json::to_vec(&request).map_err(|_| "invalid-request")?;
    if encoded.len() > mas_host_worker::MAX_FRAME_BYTES {
        return Err("request-limit");
    }
    // This command has no retained run. An anonymous descriptor supplies the
    // existing guardian's ownership lifetime without a filesystem journal.
    let owner = ownership().map_err(|_| "worker-unavailable")?;
    let arguments = vec![
        "-I".into(),
        "-B".into(),
        config.worker.to_str().ok_or("worker-unavailable")?.into(),
        "--manifest".into(),
        config.manifest.to_str().ok_or("worker-unavailable")?.into(),
        "--assets".into(),
        config.assets.to_str().ok_or("worker-unavailable")?.into(),
    ];
    let environment: BTreeMap<String, String> = [
        ("HF_HUB_OFFLINE", "1"),
        ("HF_HUB_DISABLE_TELEMETRY", "1"),
        ("TRANSFORMERS_OFFLINE", "1"),
        ("TOKENIZERS_PARALLELISM", "false"),
        ("OMP_NUM_THREADS", "1"),
        ("OPENBLAS_NUM_THREADS", "1"),
        ("MKL_NUM_THREADS", "1"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), v.into()))
    .collect();
    let deadline = mas_host_process::unix_ms()
        .checked_add(config.timeout_ms)
        .ok_or("worker-unavailable")?;
    let mut worker = Worker::spawn_scoped_with_limits(
        &config.python,
        &request,
        deadline,
        owner.as_raw_fd(),
        &arguments,
        &environment,
        Some(&config.assets),
        config.limits,
    )
    .map_err(|_| "worker-unavailable")?;
    let response = worker.read();
    // No response is accepted until the same guardian confirms descendant cleanup.
    worker.stop().map_err(|_| "worker-cleanup-failed")?;
    NeuralAssessment::from_value(
        &response.map_err(|_| "worker-incomplete")?,
        source,
        &config.pins,
        limits,
    )
    .map_err(|_| "invalid-worker-response")
}

fn ownership() -> std::io::Result<File> {
    // SAFETY: the constant C string is terminated; ownership transfers to File
    // only for a valid descriptor and is closed exactly once on every path.
    let fd = unsafe { libc::memfd_create(c"mas-privacy-owner".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let file = unsafe { File::from_raw_fd(fd) };
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}
