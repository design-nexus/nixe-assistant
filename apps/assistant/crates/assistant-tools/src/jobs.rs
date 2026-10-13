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
        anyhow::ensure!(
            !argv.is_empty() && argv.iter().all(|a| !a.contains('\0')),
            "invalid command arguments"
        );
        let cwd = crate::typed::home_path(cwd)?;
        anyhow::ensure!(cwd.is_dir(), "working directory is not a directory");
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
        let mut child = cmd.spawn().context("start terminal job")?;
        let pid = child.id().context("missing child PID")?;
        let id = format!(
            "{}-{}",
            std::process::id(),
            self.next.fetch_add(1, Ordering::Relaxed)
        );
        let data = json!({"job_id":id,"argv":argv,"cwd":cwd,"status":"running","exit_code":null,"output":""});
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
}
