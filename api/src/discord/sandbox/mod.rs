//! Linux sandboxes the Discord agent runs code in, one per channel: rootless podman containers on
//! the host, driven through podman's Docker-compatible API. A container is disposable. It stops
//! once its channel leaves it idle and is recreated when its image changes, while its home
//! directory lives on in a volume until the sandbox goes unused for `SANDBOX_RETENTION`.
//!
//! What keeps a sandbox in its box (no route to the host or private networks, memory and disk
//! caps shared by all of them) is set up on the host, in the NixOS config that runs podman.

mod output;
mod store;

use std::{
    collections::HashMap,
    fmt,
    io::Read as _,
    num::NonZeroU64,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::{Duration, Instant},
};

use bollard::{
    ClientVersion, Docker,
    errors::Error as DockerError,
    exec::{CreateExecOptions, StartExecResults},
    models::{ContainerCreateBody, HostConfig, Mount, MountType, VolumeCreateRequest},
    query_parameters::{
        CreateContainerOptionsBuilder, DownloadFromContainerOptions, ListContainersOptions,
        ListVolumesOptions, RemoveContainerOptions, RemoveVolumeOptions, StopContainerOptions,
        UploadToContainerOptions,
    },
};
use chrono::Utc;
use eyre::{Context as _, bail, eyre};
use futures::StreamExt as _;
use serenity::all::ChannelId;
use tracing::Instrument as _;

use crate::discord::{
    chatgpt::DbPool,
    constants::{SANDBOX_IDLE_TIMEOUT, SANDBOX_RETENTION, SANDBOX_SWEEP_INTERVAL},
};
use output::Capture;
use store::SandboxStore;

/// The sandbox's persistent directory, where commands start
pub const HOME: &str = "/home/bot";
/// Marks the containers and volumes that are sandboxes
const MANAGED_LABEL: &str = "sh.wrx.sandbox";
/// The channel a sandbox's container or volume belongs to
const CHANNEL_LABEL: &str = "sh.wrx.sandbox.channel";
const MEMORY_BYTES: i64 = 1024 * 1024 * 1024;
const NANO_CPUS: i64 = 1_500_000_000;
const PIDS_LIMIT: i64 = 512;
/// The host's own resolver is out of the sandboxes' reach, so they use public ones
const DNS_SERVERS: [&str; 2] = ["1.1.1.1", "9.9.9.9"];
/// The newest Docker API version podman speaks
const API_VERSION: ClientVersion = ClientVersion {
    major_version: 1,
    minor_version: 41,
};
/// Bounds each API call until its response starts, not a command's output streaming after it
const API_TIMEOUT_SECS: u64 = 120;
/// How long a stopping sandbox gets to exit before it's killed
const STOP_GRACE_SECS: i32 = 5;
/// How long past its timeout a command's output is waited for, while `timeout` kills it
const EXEC_GRACE: Duration = Duration::from_secs(15);
/// How soon the janitor tries again while podman is out of reach
const JANITOR_RETRY: Duration = Duration::from_secs(60);

/// Runs `$2` in a fresh bash under `timeout $1`, stdout and stderr merged, streaming its output
/// back while `tee` keeps all of it in the log file `$3`. Logs over a day old are dropped.
const RUN_SCRIPT: &str = r#"log_dir=$(dirname "$3")
mkdir -p "$log_dir" && find "$log_dir" -name '*.log' -mtime +0 -delete
timeout --kill-after=5 "$1" bash -c "$2" < /dev/null 2>&1 | tee "$3"
exit "${PIPESTATUS[0]}""#;

/// The sandboxes of every channel
#[derive(Clone)]
pub struct Sandboxes(Arc<Shared>);

struct Shared {
    docker: Docker,
    /// What new containers are created from
    image: String,
    store: SandboxStore,
    /// Each channel's `Lifecycle`, created on first use
    lifecycles: Mutex<HashMap<ChannelId, Arc<tokio::sync::Mutex<Lifecycle>>>>,
}

/// What this process knows of a channel's sandbox. Held locked while the sandbox is started,
/// stopped, or deleted, so the channel and the janitor never do so at once.
#[derive(Default)]
struct Lifecycle {
    /// This process started the sandbox or took it over, and stops it once it goes idle. The
    /// janitor leaves it alone.
    claimed: bool,
}

impl Sandboxes {
    /// Talks to podman at `socket`, which is first tried on use, and starts the janitor
    pub fn start(socket: &str, image: String, db: DbPool) -> eyre::Result<Self> {
        let docker = Docker::connect_with_unix(socket, API_TIMEOUT_SECS, &API_VERSION)
            .wrap_err_with(|| format!("Failed to set up the podman client for {socket}"))?;
        let sandboxes = Self(Arc::new(Shared {
            docker,
            image,
            store: SandboxStore::new(db),
            lifecycles: Mutex::default(),
        }));
        tokio::spawn(
            sandboxes
                .clone()
                .janitor()
                .instrument(tracing::info_span!("sandbox_janitor")),
        );
        Ok(sandboxes)
    }

    /// The sandbox of `channel_id`. A channel takes one and keeps it: it tracks when the sandbox
    /// was last used.
    pub fn channel(&self, channel_id: ChannelId) -> ChannelSandbox {
        ChannelSandbox {
            sandboxes: self.clone(),
            channel_id,
            lifecycle: self.lifecycle(channel_id),
            last_used: Arc::default(),
        }
    }

    fn docker(&self) -> &Docker {
        &self.0.docker
    }

    fn lifecycle(&self, channel_id: ChannelId) -> Arc<tokio::sync::Mutex<Lifecycle>> {
        self.0
            .lifecycles
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(channel_id)
            .or_default()
            .clone()
    }

    async fn image_id(&self) -> eyre::Result<String> {
        let image = &self.0.image;
        match self.docker().inspect_image(image).await {
            Ok(inspect) => inspect
                .id
                .as_deref()
                .map(bare_id)
                .ok_or_else(|| eyre!("podman reported no ID for the sandbox image {image}")),
            Err(e) if is_status(&e, 404) => Err(eyre!(
                "the sandbox image {image} isn't loaded on the host; nothing on your side can fix that"
            )),
            Err(e) => Err(e).wrap_err("Failed to look up the sandbox image"),
        }
    }

    /// Stops the sandboxes a previous process left running, which nobody would stop otherwise,
    /// then deletes the ones unused for `SANDBOX_RETENTION`, every `SANDBOX_SWEEP_INTERVAL`
    async fn janitor(self) {
        let mut strays_stopped = false;
        loop {
            if !strays_stopped {
                match self.stop_strays().await {
                    Ok(()) => strays_stopped = true,
                    Err(e) => tracing::warn!(?e, "Failed to stop the sandboxes left running"),
                }
            }
            if strays_stopped && let Err(e) = self.delete_unused().await {
                tracing::error!(?e, "Failed to delete the unused sandboxes");
            }
            tokio::time::sleep(if strays_stopped {
                SANDBOX_SWEEP_INTERVAL
            } else {
                JANITOR_RETRY
            })
            .await;
        }
    }

    async fn stop_strays(&self) -> eyre::Result<()> {
        let running = self
            .docker()
            .list_containers(Some(ListContainersOptions {
                filters: Some(managed_filter()),
                ..Default::default()
            }))
            .await
            .wrap_err("Failed to list the running sandboxes")?;
        for container in running {
            let (Some(id), Some(channel_id)) = (
                container.id,
                container.labels.as_ref().and_then(channel_of),
            ) else {
                continue;
            };
            let lifecycle = self.lifecycle(channel_id);
            let lifecycle = lifecycle.lock().await;
            if lifecycle.claimed {
                continue;
            }
            let stop = StopContainerOptions {
                t: Some(STOP_GRACE_SECS),
                signal: None,
            };
            match self.docker().stop_container(&id, Some(stop)).await {
                Ok(()) => {}
                Err(e) if is_status(&e, 304) || is_status(&e, 404) => {}
                Err(e) => return Err(e).wrap_err("Failed to stop a sandbox left running"),
            }
            if let Err(e) = self.0.store.touch(channel_id).await {
                tracing::error!(?e, "Failed to record when a sandbox was stopped");
            }
            tracing::info!(
                channel_id = channel_id.get(),
                "Stopped a sandbox left running from before"
            );
        }
        Ok(())
    }

    async fn delete_unused(&self) -> eyre::Result<()> {
        // A sandbox missing from the table, like one whose use failed to be recorded, gets its
        // full retention from now on
        let volumes = self
            .docker()
            .list_volumes(Some(ListVolumesOptions {
                filters: Some(managed_filter()),
            }))
            .await
            .wrap_err("Failed to list the sandbox volumes")?;
        for volume in volumes.volumes.unwrap_or_default() {
            if let Some(channel_id) = channel_of(&volume.labels) {
                self.0.store.adopt(channel_id).await?;
            }
        }

        let retention =
            chrono::Duration::from_std(SANDBOX_RETENTION).wrap_err("Unusable sandbox retention")?;
        for channel_id in self.0.store.unused_since(Utc::now() - retention).await? {
            let lifecycle = self.lifecycle(channel_id);
            let lifecycle = lifecycle.lock().await;
            if lifecycle.claimed {
                continue;
            }
            let name = sandbox_name(channel_id);
            match self
                .docker()
                .remove_container(&name, None::<RemoveContainerOptions>)
                .await
            {
                Ok(()) => {}
                Err(e) if is_status(&e, 404) => {}
                Err(e) => {
                    tracing::warn!(?e, %name, "Failed to delete an unused sandbox's container");
                    continue;
                }
            }
            match self
                .docker()
                .remove_volume(&name, None::<RemoveVolumeOptions>)
                .await
            {
                Ok(()) => {}
                Err(e) if is_status(&e, 404) => {}
                Err(e) => {
                    tracing::warn!(?e, %name, "Failed to delete an unused sandbox's volume");
                    continue;
                }
            }
            self.0.store.forget(channel_id).await?;
            tracing::info!(channel_id = channel_id.get(), "Deleted an unused sandbox");
        }
        Ok(())
    }
}

/// A channel's sandbox
#[derive(Clone)]
pub struct ChannelSandbox {
    sandboxes: Sandboxes,
    channel_id: ChannelId,
    lifecycle: Arc<tokio::sync::Mutex<Lifecycle>>,
    /// When a tool last used it; `None` while it's stopped, as far as this process knows
    last_used: Arc<Mutex<Option<Instant>>>,
}

impl fmt::Debug for ChannelSandbox {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChannelSandbox")
            .field("channel_id", &self.channel_id)
            .finish_non_exhaustive()
    }
}

/// What a command printed and how it ended
pub struct RunOutput {
    /// `None` when its output outlasted its timeout, so the call stopped waiting
    pub exit_code: Option<i64>,
    /// Its stdout and stderr as they came, the middle cut out when it's long
    pub output: String,
    /// Bytes cut out of `output`
    pub omitted_bytes: usize,
    /// Where the whole output is kept in the sandbox
    pub log_path: String,
}

/// A file read out of the sandbox
pub struct SandboxFile {
    pub name: String,
    pub bytes: Vec<u8>,
}

impl ChannelSandbox {
    fn name(&self) -> String {
        sandbox_name(self.channel_id)
    }

    fn docker(&self) -> &Docker {
        self.sandboxes.docker()
    }

    fn last_used(&self) -> MutexGuard<'_, Option<Instant>> {
        self.last_used.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// When the running sandbox goes idle and is due to stop
    pub fn idle_deadline(&self) -> Option<Instant> {
        self.last_used().map(|at| at + SANDBOX_IDLE_TIMEOUT)
    }

    /// Stops the sandbox. Its home directory stays for the next start.
    pub async fn stop(&self) {
        let mut lifecycle = self.lifecycle.lock().await;
        let stop = StopContainerOptions {
            t: Some(STOP_GRACE_SECS),
            signal: None,
        };
        match self.docker().stop_container(&self.name(), Some(stop)).await {
            Ok(()) => tracing::info!("Stopped the idle sandbox"),
            Err(e) if is_status(&e, 304) || is_status(&e, 404) => {}
            Err(e) => {
                tracing::error!(?e, "Failed to stop the idle sandbox; trying again later");
                *self.last_used() = Some(Instant::now());
                return;
            }
        }
        *self.last_used() = None;
        lifecycle.claimed = false;
        if let Err(e) = self.sandboxes.0.store.touch(self.channel_id).await {
            tracing::error!(?e, "Failed to record when the sandbox was stopped");
        }
    }

    /// Makes sure the sandbox runs: starts it, first creating it when it's missing or its image
    /// has changed since
    async fn ready(&self) -> eyre::Result<()> {
        let mut lifecycle = self.lifecycle.lock().await;
        let name = self.name();
        let running = match self.docker().inspect_container(&name, None).await {
            Ok(container) => {
                let running = container
                    .state
                    .as_ref()
                    .and_then(|state| state.running)
                    .unwrap_or(false);
                if !running
                    && container.image.as_deref().map(bare_id)
                        != Some(self.sandboxes.image_id().await?)
                {
                    tracing::info!("Recreating the sandbox on its new image");
                    self.docker()
                        .remove_container(&name, None::<RemoveContainerOptions>)
                        .await
                        .wrap_err("Failed to remove the sandbox's outdated container")?;
                    self.create().await?;
                }
                running
            }
            Err(e) if is_status(&e, 404) => {
                // Surfaces a missing image before creating anything
                self.sandboxes.image_id().await?;
                self.create().await?;
                false
            }
            Err(e) => return Err(e).wrap_err("Failed to look up the sandbox"),
        };

        // Claimed before it starts, so the janitor never takes it for a stray, and due for its
        // idle stop from now on, which also cleans up after a start that fails halfway
        lifecycle.claimed = true;
        *self.last_used() = Some(Instant::now());
        if !running {
            self.docker()
                .start_container(&name, None)
                .await
                .wrap_err("Failed to start the sandbox")?;
            tracing::info!("Started the sandbox");
            if let Err(e) = self.sandboxes.0.store.touch(self.channel_id).await {
                tracing::error!(?e, "Failed to record when the sandbox was started");
            }
        }
        Ok(())
    }

    async fn create(&self) -> eyre::Result<()> {
        let name = self.name();
        let labels = HashMap::from([
            (MANAGED_LABEL.to_string(), "true".to_string()),
            (CHANNEL_LABEL.to_string(), self.channel_id.to_string()),
        ]);

        // Created up front, where mounting would create it bare, to carry the labels the janitor
        // finds it by
        let volume = VolumeCreateRequest {
            name: Some(name.clone()),
            labels: Some(labels.clone()),
            ..Default::default()
        };
        match self.docker().create_volume(volume).await {
            Ok(_) => {}
            Err(e) if is_status(&e, 409) => {}
            Err(e) => return Err(e).wrap_err("Failed to create the sandbox's home volume"),
        }

        let container = ContainerCreateBody {
            image: Some(self.sandboxes.0.image.clone()),
            hostname: Some("sandbox".to_string()),
            labels: Some(labels),
            host_config: Some(HostConfig {
                memory: Some(MEMORY_BYTES),
                memory_swap: Some(MEMORY_BYTES),
                nano_cpus: Some(NANO_CPUS),
                pids_limit: Some(PIDS_LIMIT),
                cap_drop: Some(vec!["ALL".to_string()]),
                security_opt: Some(vec!["no-new-privileges".to_string()]),
                // Its own network namespace, so the sandboxes can't reach each other
                network_mode: Some("pasta".to_string()),
                dns: Some(DNS_SERVERS.iter().map(ToString::to_string).collect()),
                mounts: Some(vec![Mount {
                    target: Some(HOME.to_string()),
                    source: Some(name.clone()),
                    typ: Some(MountType::VOLUME),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };
        self.docker()
            .create_container(
                Some(CreateContainerOptionsBuilder::default().name(&name).build()),
                container,
            )
            .await
            .wrap_err("Failed to create the sandbox")?;
        tracing::info!("Created the sandbox");
        Ok(())
    }

    /// Runs `command` in bash from `HOME`, killing it after `timeout`
    pub async fn run(&self, command: &str, timeout: Duration) -> eyre::Result<RunOutput> {
        self.ready().await?;
        let log_path = format!(
            "{HOME}/.logs/{}.log",
            Utc::now().format("%Y-%m-%d_%H-%M-%S%.3f")
        );
        let timeout_secs = timeout.as_secs().to_string();
        let exec = self
            .docker()
            .create_exec(
                &self.name(),
                CreateExecOptions {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    working_dir: Some(HOME),
                    cmd: Some(vec![
                        "bash",
                        "-c",
                        RUN_SCRIPT,
                        "sandbox_run",
                        &timeout_secs,
                        command,
                        &log_path,
                    ]),
                    ..Default::default()
                },
            )
            .await
            .wrap_err("Failed to set up the command")?;

        let mut capture = Capture::default();
        let started = self
            .docker()
            .start_exec(&exec.id, None)
            .await
            .wrap_err("Failed to start the command")?;
        let finished = match started {
            StartExecResults::Attached { mut output, .. } => {
                let read = async {
                    while let Some(chunk) = output.next().await {
                        capture.push(&chunk?.into_bytes());
                    }
                    Ok::<_, DockerError>(())
                };
                match tokio::time::timeout(timeout + EXEC_GRACE, read).await {
                    Ok(read) => {
                        read.wrap_err("Lost the command's output")?;
                        true
                    }
                    // Something holds its output open, like a background process started without
                    // redirecting it
                    Err(_) => false,
                }
            }
            StartExecResults::Detached => bail!("podman ran the command detached"),
        };
        *self.last_used() = Some(Instant::now());

        let exit_code = if finished {
            self.exit_code(&exec.id).await
        } else {
            None
        };
        let (output, omitted_bytes) = capture.finish();
        Ok(RunOutput {
            exit_code,
            output,
            omitted_bytes,
            log_path,
        })
    }

    /// Writes `contents` to the file at `path`, creating its directory. Returns the absolute
    /// path.
    pub async fn write_file(&self, path: &str, contents: &[u8]) -> eyre::Result<String> {
        let path = absolute(path)?;
        let (directory, file_name) = split_path(&path)?;
        self.ready().await?;
        self.exec_checked(vec!["mkdir", "-p", "--", directory])
            .await?;

        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(u64::try_from(contents.len()).wrap_err("The file is too large")?);
        header.set_mode(0o644);
        header.set_mtime(Utc::now().timestamp().cast_unsigned());
        let mut archive = tar::Builder::new(Vec::new());
        archive
            .append_data(&mut header, file_name, contents)
            .wrap_err("Failed to pack the file")?;
        let archive = archive.into_inner().wrap_err("Failed to pack the file")?;

        self.docker()
            .upload_to_container(
                &self.name(),
                Some(UploadToContainerOptions {
                    path: directory.to_string(),
                    ..Default::default()
                }),
                bollard::body_full(archive.into()),
            )
            .await
            .wrap_err_with(|| format!("Failed to write {path}"))?;
        *self.last_used() = Some(Instant::now());
        Ok(path)
    }

    /// Reads the file at `path`, failing when it holds more than `max_bytes`
    pub async fn read_file(&self, path: &str, max_bytes: u64) -> eyre::Result<SandboxFile> {
        let path = absolute(path)?;
        let (_, file_name) = split_path(&path)?;
        self.ready().await?;

        // The file comes as a tar archive, a few headers bigger than the file itself
        let limit = usize::try_from(max_bytes)
            .unwrap_or(usize::MAX)
            .saturating_add(64 * 1024);
        let mut archive = Vec::new();
        let mut download = self.docker().download_from_container(
            &self.name(),
            Some(DownloadFromContainerOptions { path: path.clone() }),
        );
        while let Some(chunk) = download.next().await {
            let chunk = chunk.map_err(|e| {
                if is_status(&e, 404) {
                    eyre!("{path} doesn't exist")
                } else {
                    eyre!(e).wrap_err(format!("Failed to read {path}"))
                }
            })?;
            archive.extend_from_slice(&chunk);
            if archive.len() > limit {
                bail!("{path} is over the {max_bytes} byte limit");
            }
        }
        *self.last_used() = Some(Instant::now());

        let bytes = file_from_archive(&archive, &path)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max_bytes {
            bail!("{path} is over the {max_bytes} byte limit");
        }
        Ok(SandboxFile {
            name: file_name.to_string(),
            bytes,
        })
    }

    /// Runs `cmd` to the end, failing with what it printed unless it exits 0
    async fn exec_checked(&self, cmd: Vec<&str>) -> eyre::Result<()> {
        let description = cmd.join(" ");
        let exec = self
            .docker()
            .create_exec(
                &self.name(),
                CreateExecOptions {
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    cmd: Some(cmd),
                    ..Default::default()
                },
            )
            .await
            .wrap_err_with(|| format!("Failed to run `{description}`"))?;
        let mut capture = Capture::default();
        if let StartExecResults::Attached { mut output, .. } = self
            .docker()
            .start_exec(&exec.id, None)
            .await
            .wrap_err_with(|| format!("Failed to run `{description}`"))?
        {
            while let Some(chunk) = output.next().await {
                let chunk = chunk.wrap_err_with(|| format!("Failed to run `{description}`"))?;
                capture.push(&chunk.into_bytes());
            }
        }
        match self.exit_code(&exec.id).await {
            Some(0) => Ok(()),
            code => {
                let (output, _) = capture.finish();
                let code = code.map_or_else(|| "unknown".to_string(), |code| code.to_string());
                bail!(
                    "`{description}` failed with exit code {code}: {}",
                    output.trim()
                )
            }
        }
    }

    /// The exit code of an exec whose output has ended. Podman can take a moment to record it.
    async fn exit_code(&self, exec_id: &str) -> Option<i64> {
        for _ in 0..20 {
            match self.docker().inspect_exec(exec_id).await {
                Ok(exec) if exec.running != Some(true) => return exec.exit_code,
                Ok(_) => tokio::time::sleep(Duration::from_millis(100)).await,
                Err(e) => {
                    tracing::warn!(?e, "Failed to look up how a command ended");
                    return None;
                }
            }
        }
        None
    }
}

fn sandbox_name(channel_id: ChannelId) -> String {
    format!("wrx-sbx-{channel_id}")
}

fn managed_filter() -> HashMap<String, Vec<String>> {
    HashMap::from([("label".to_string(), vec![MANAGED_LABEL.to_string()])])
}

fn channel_of(labels: &HashMap<String, String>) -> Option<ChannelId> {
    labels
        .get(CHANNEL_LABEL)?
        .parse::<NonZeroU64>()
        .ok()
        .map(ChannelId::from)
}

/// An image ID without the `sha256:` podman adds in some places and not others
fn bare_id(id: &str) -> String {
    id.trim_start_matches("sha256:").to_string()
}

fn is_status(error: &DockerError, status: u16) -> bool {
    matches!(
        error,
        DockerError::DockerResponseServerError { status_code, .. } if *status_code == status
    )
}

/// `path` as an absolute path in the sandbox, where relative paths and `~` start at `HOME`
fn absolute(path: &str) -> eyre::Result<String> {
    let path = path.trim();
    if path.is_empty() {
        bail!("the path is empty");
    }
    Ok(if path == "~" {
        HOME.to_string()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("{HOME}/{rest}")
    } else if path.starts_with('/') {
        path.to_string()
    } else {
        format!("{HOME}/{path}")
    })
}

/// The directory and file name of an absolute file path
fn split_path(path: &str) -> eyre::Result<(&str, &str)> {
    let as_path = Path::new(path);
    let file_name = as_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| eyre!("{path} doesn't name a file"))?;
    let directory = as_path
        .parent()
        .and_then(|directory| directory.to_str())
        .filter(|directory| !directory.is_empty())
        .ok_or_else(|| eyre!("{path} doesn't name a file"))?;
    Ok((directory, file_name))
}

/// The contents of the one file in a tar `archive` of `path`
fn file_from_archive(archive: &[u8], path: &str) -> eyre::Result<Vec<u8>> {
    let mut archive = tar::Archive::new(archive);
    let mut entries = archive
        .entries()
        .wrap_err_with(|| format!("Failed to unpack {path}"))?;
    let mut entry = entries
        .next()
        .ok_or_else(|| eyre!("{path} came back empty"))?
        .wrap_err_with(|| format!("Failed to unpack {path}"))?;
    let kind = entry.header().entry_type();
    if kind.is_dir() {
        bail!("{path} is a directory");
    }
    if kind.is_symlink() {
        let target = entry
            .link_name()
            .ok()
            .flatten()
            .map(|target| target.display().to_string())
            .unwrap_or_default();
        bail!("{path} is a symlink to {target}; use the path of the file it points to");
    }
    if !kind.is_file() {
        bail!("{path} isn't a regular file");
    }
    let mut bytes = Vec::new();
    entry
        .read_to_end(&mut bytes)
        .wrap_err_with(|| format!("Failed to unpack {path}"))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_start_at_home() {
        for (path, expected) in [
            ("plot.png", "/home/bot/plot.png"),
            (" out/plot.png ", "/home/bot/out/plot.png"),
            ("~/plot.png", "/home/bot/plot.png"),
            ("~", "/home/bot"),
            ("/tmp/plot.png", "/tmp/plot.png"),
        ] {
            assert_eq!(absolute(path).ok().as_deref(), Some(expected), "{path}");
        }
        assert!(absolute("  ").is_err());
    }

    #[test]
    fn paths_split_into_directory_and_file() {
        assert_eq!(
            split_path("/home/bot/out/plot.png").ok(),
            Some(("/home/bot/out", "plot.png"))
        );
        assert_eq!(split_path("/plot.png").ok(), Some(("/", "plot.png")));
        assert!(split_path("/").is_err());
    }

    fn archive(entries: &[(&str, tar::EntryType, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(Vec::new());
        for (name, kind, data) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            builder
                .append_data(&mut header, name, *data)
                .expect("append to the test archive");
        }
        builder.into_inner().expect("finish the test archive")
    }

    #[test]
    fn a_file_comes_out_of_its_archive() {
        let tar = archive(&[("plot.png", tar::EntryType::Regular, b"png bytes")]);
        assert_eq!(
            file_from_archive(&tar, "/home/bot/plot.png").ok(),
            Some(b"png bytes".to_vec())
        );

        let tar = archive(&[("out", tar::EntryType::Directory, b"")]);
        let error = file_from_archive(&tar, "/home/bot/out").expect_err("a directory");
        assert!(error.to_string().contains("is a directory"));
    }

    #[test]
    fn channels_come_from_labels() {
        let labels = HashMap::from([(CHANNEL_LABEL.to_string(), "123".to_string())]);
        assert_eq!(channel_of(&labels), Some(ChannelId::new(123)));
        let labels = HashMap::from([(CHANNEL_LABEL.to_string(), "0".to_string())]);
        assert_eq!(channel_of(&labels), None);
    }
}
