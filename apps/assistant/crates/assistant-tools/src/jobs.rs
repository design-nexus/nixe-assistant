use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::Mutex,
};
use tokio_util::sync::CancellationToken;
#[derive(Default)]
pub struct Jobs {
    next: AtomicU64,
    entries: Mutex<BTreeMap<String, Arc<Job>>>,
}
struct Job {
    data: Mutex<Value>,
    cancel: CancellationToken,
    output: Mutex<Vec<u8>>,
}
impl Jobs {
    pub async fn start(&self, argv: Vec<String>, cwd: &str, timeout: u64) -> Result<Value> {
        self.start_in(argv, cwd, timeout, None).await
    }
    /// Start a job, inside `sandbox` when given. A sandboxed job writes only to its working
    /// folder, which can't be HOME itself or a private folder; asking for HOME gets a fresh
    /// scratch folder instead.
    pub async fn start_in(
        &self,
        argv: Vec<String>,
        cwd: &str,
        timeout: u64,
        sandbox: Option<&crate::sandbox::Sandbox>,
    ) -> Result<Value> {
        anyhow::ensure!(
            !argv.is_empty() && argv.iter().all(|a| !a.contains('\0')),
            "invalid command arguments"
        );
        let mut cwd = crate::typed::home_path(cwd)?;
        anyhow::ensure!(cwd.is_dir(), "working directory is not a directory");
        let shown = argv.clone();
        // The seccomp program for a sandboxed job; bwrap reads it, the daemon closes it.
        let mut seccomp = None;
        let argv = match sandbox {
            None => argv,
            Some(sandbox) => {
                let home = crate::fs::home()?;
                if cwd == home {
                    let scratch = assistant_core::paths::cache_dir().join("sandbox");
                    prune_scratch(&scratch);
                    cwd = scratch.join(format!("{}-{}", std::process::id(), self.next.load(Ordering::Relaxed)));
                    std::fs::create_dir_all(&cwd)?;
                    cwd = cwd.canonicalize()?;
                }
                anyhow::ensure!(
                    crate::fs::sandbox_dir_ok(&home, &cwd),
                    "a sandboxed command needs a project folder; HOME itself and private folders can't be shared"
                );
                let mut argv = crate::sandbox::wrap(sandbox, &home, &cwd, argv);
                let fd = crate::plugin_sandbox::seccomp_fd()?;
                argv.splice(1..1, ["--seccomp".to_string(), std::os::fd::AsRawFd::as_raw_fd(&fd).to_string()]);
                seccomp = Some(fd);
                // A build may use every core; memory and processes stay bounded.
                let cores = std::thread::available_parallelism().map(|n| n.get() as u64).unwrap_or(4);
                let limits = assistant_core::config::SandboxProfile { memory_mb: Some(8192), tasks: Some(2048), cpu_percent: Some((cores * 100).min(3200)), ..Default::default() };
                let unit = format!("nixe-assistant-job-{}-{}.scope", std::process::id(), self.next.load(Ordering::Relaxed));
                let mut limited = crate::plugin_sandbox::limits(&limits, &unit);
                limited.extend(argv);
                limited
            }
        };
        let mut entries = self.entries.lock().await;
        anyhow::ensure!(
            entries
                .values()
                .filter(|j| j.data.try_lock().is_ok_and(|d| d["status"] == "running"))
                .count()
                < 8,
            "too many running terminal jobs"
        );
        let mut cmd = tokio::process::Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .current_dir(&cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .process_group(0);
        let parent = std::process::id();
        unsafe {
            cmd.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != parent as i32 {
                    return Err(std::io::Error::other("job owner exited"));
                }
                Ok(())
            });
        }
        if let Some(fd) = &seccomp {
            crate::plugin_sandbox::inherit(&mut cmd, vec![std::os::fd::AsRawFd::as_raw_fd(fd)]);
        }
        let mut child = cmd.spawn().context("start terminal job")?;
        drop(seccomp);
        let pid = child.id().context("missing child PID")?;
        let id = format!(
            "{}-{}",
            std::process::id(),
            self.next.fetch_add(1, Ordering::Relaxed)
        );
        let data = json!({"job_id":id,"argv":shown,"cwd":cwd,"sandboxed":sandbox.is_some(),"status":"running","exit_code":null,"output":""});
        let job = Arc::new(Job {
            data: Mutex::new(data.clone()),
            cancel: CancellationToken::new(),
            output: Mutex::new(Vec::new()),
        });
        let out = child.stdout.take().unwrap();
        let err = child.stderr.take().unwrap();
        {
            if entries.len() >= 100 {
                let oldest = entries
                    .iter()
                    .find(|(_, j)| j.data.try_lock().is_ok_and(|d| d["status"] != "running"))
                    .map(|(id, _)| id.clone());
                if let Some(id) = oldest {
                    entries.remove(&id);
                }
            }
            entries.insert(id.clone(), job.clone());
        }
        drop(entries);
        let j = job.clone();
        let a = tokio::spawn(drain(out, j.clone()));
        let b = tokio::spawn(drain(err, j.clone()));
        tokio::spawn(async move {
            let (status, reason) = tokio::select! {result=child.wait()=>(result,"finished"),_=j.cancel.cancelled()=>{terminate(pid);let _=child.kill().await;(child.wait().await,"cancelled")},_=tokio::time::sleep(Duration::from_secs(timeout.clamp(1,600)))=>{terminate(pid);let _=child.kill().await;(child.wait().await,"timed_out")}};
            // A successful parent may leave descendants holding pipes. Bound draining and close them.
            terminate(pid);
            let _ = tokio::time::timeout(Duration::from_secs(1), async {
                let _ = a.await;
                let _ = b.await;
            })
            .await;
            let mut d = j.data.lock().await;
            d["status"] = reason.into();
            d["exit_code"] = status.ok().and_then(|s| s.code()).into();
        });
        Ok(data)
    }
    pub async fn status(&self, id: &str) -> Result<Value> {
        let job = self
            .entries
            .lock()
            .await
            .get(id)
            .cloned()
            .context("unknown terminal job")?;
        let mut data = job.data.lock().await.clone();
        data["output"] = String::from_utf8_lossy(&job.output.lock().await)
            .into_owned()
            .into();
        Ok(data)
    }
    pub async fn cancel(&self, id: &str) -> Result<Value> {
        let job = self
            .entries
            .lock()
            .await
            .get(id)
            .cloned()
            .context("unknown terminal job")?;
        job.cancel.cancel();
        Ok(json!({"job_id":id,"cancelled":true}))
    }
    pub async fn cancel_all(&self) {
        for j in self.entries.lock().await.values() {
            j.cancel.cancel();
        }
    }
}
fn terminate(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}
async fn drain(mut stream: impl AsyncRead + Unpin, job: Arc<Job>) {
    let mut buf = [0; 4096];
    while let Ok(n) = stream.read(&mut buf).await {
        if n == 0 {
            break;
        }
        let mut data = job.output.lock().await;
        data.extend_from_slice(&buf[..n]);
        if data.len() > 65536 {
            let excess = data.len() - 65536;
            data.drain(..excess);
        }
    }
}
/// Scratch folders of sandboxed jobs are kept a day for their output files, then removed.
fn prune_scratch(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let day = Duration::from_secs(24 * 3600);
    for entry in entries.flatten() {
        let old = entry.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > day);
        if old && entry.file_type().is_ok_and(|t| t.is_dir()) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn arguments_timeout_and_cancellation() {
        let jobs = Jobs::default();
        let home = std::env::var("HOME").unwrap();
        let v = jobs
            .start(
                vec!["printf".into(), "%s".into(), "space ' quote\nline".into()],
                &home,
                2,
            )
            .await
            .unwrap();
        let id = v["job_id"].as_str().unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            jobs.status(id).await.unwrap()["output"],
            "space ' quote\nline"
        );
        let v = jobs
            .start(vec!["sleep".into(), "30".into()], &home, 1)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert_eq!(
            jobs.status(v["job_id"].as_str().unwrap()).await.unwrap()["status"],
            "timed_out"
        );
        let v = jobs
            .start(vec!["sleep".into(), "30".into()], &home, 30)
            .await
            .unwrap();
        jobs.cancel(v["job_id"].as_str().unwrap()).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(
            jobs.status(v["job_id"].as_str().unwrap()).await.unwrap()["status"],
            "cancelled"
        );
    }

    /// A sandboxed job runs under the seccomp filter and in a limited scope: it writes its
    /// project folder, and can't make new namespaces.
    #[tokio::test]
    async fn sandboxed_jobs_are_filtered() {
        if std::process::Command::new("bwrap").args(["--ro-bind", "/", "/", "true"]).status().map(|s| !s.success()).unwrap_or(true) {
            return;
        }
        let home = crate::fs::home().unwrap();
        let cache = home.join(".cache");
        std::fs::create_dir_all(&cache).unwrap();
        let project = tempfile::tempdir_in(&cache).unwrap();
        let sandbox = crate::sandbox::Sandbox { network: false, read_only: vec![] };
        let jobs = Jobs::default();
        let script = "echo made > out.txt; if unshare -r true 2>/dev/null; then echo escaped; else echo refused; fi";
        let v = jobs
            .start_in(vec!["sh".into(), "-c".into(), script.into()], &project.path().display().to_string(), 20, Some(&sandbox))
            .await
            .unwrap();
        let id = v["job_id"].as_str().unwrap().to_string();
        let mut status = Value::Null;
        for _ in 0..100 {
            status = jobs.status(&id).await.unwrap();
            if status["status"] != "running" {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(status["status"], "finished", "{status}");
        assert_eq!(status["output"].as_str().map(str::trim), Some("refused"));
        assert_eq!(std::fs::read_to_string(project.path().join("out.txt")).unwrap().trim(), "made");
    }
}
