use std::collections::HashMap;
use std::process::Child;
use std::sync::{Arc, Mutex, OnceLock, Weak};

#[derive(Debug)]
pub struct ManagedProcess {
    key: String,
    signature: String,
    child: Mutex<Option<Child>>,
}

impl ManagedProcess {
    pub fn pid(&self) -> Option<u32> {
        self.child
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(Child::id))
    }

    fn is_alive(&self) -> bool {
        let Ok(mut guard) = self.child.lock() else {
            return false;
        };
        let Some(child) = guard.as_mut() else {
            return false;
        };
        matches!(child.try_wait(), Ok(None))
    }
}

impl Drop for ManagedProcess {
    fn drop(&mut self) {
        let Ok(mut guard) = self.child.lock() else {
            return;
        };
        let Some(mut child) = guard.take() else {
            return;
        };
        tracing::info!(key = %self.key, pid = child.id(), "stopping managed local TTS process");
        if let Err(error) = child.kill() {
            tracing::warn!(key = %self.key, pid = child.id(), %error, "failed to kill managed local TTS process");
        }
        if let Err(error) = child.wait() {
            tracing::warn!(key = %self.key, pid = child.id(), %error, "failed to reap managed local TTS process");
        }
    }
}

fn registry() -> &'static Mutex<HashMap<String, Weak<ManagedProcess>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Weak<ManagedProcess>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn prune_dead_entries(entries: &mut HashMap<String, Weak<ManagedProcess>>) {
    entries.retain(|_, process| process.strong_count() > 0);
}

pub fn existing(key: &str, signature: &str) -> anyhow::Result<Option<Arc<ManagedProcess>>> {
    let candidate = {
        let mut entries = registry()
            .lock()
            .map_err(|_| anyhow::anyhow!("local process registry lock poisoned"))?;
        prune_dead_entries(&mut entries);
        entries.get(key).and_then(Weak::upgrade)
    };

    let Some(process) = candidate else {
        return Ok(None);
    };
    if process.is_alive() {
        if process.signature != signature {
            anyhow::bail!(
                "managed process '{key}' is already running with a different launch configuration"
            );
        }
        return Ok(Some(process));
    }

    let weak = Arc::downgrade(&process);
    let mut entries = registry()
        .lock()
        .map_err(|_| anyhow::anyhow!("local process registry lock poisoned"))?;
    if entries
        .get(key)
        .is_some_and(|current| Weak::ptr_eq(current, &weak))
    {
        entries.remove(key);
    }
    Ok(None)
}

pub fn spawn_managed<F>(
    key: impl Into<String>,
    signature: impl Into<String>,
    spawn: F,
) -> anyhow::Result<Arc<ManagedProcess>>
where
    F: FnOnce() -> anyhow::Result<Child>,
{
    let key = key.into();
    let signature = signature.into();
    let mut entries = registry()
        .lock()
        .map_err(|_| anyhow::anyhow!("local process registry lock poisoned"))?;
    prune_dead_entries(&mut entries);

    if let Some(process) = entries.get(&key).and_then(Weak::upgrade) {
        if process.is_alive() {
            if process.signature != signature {
                anyhow::bail!(
                    "managed process '{key}' is already running with a different launch configuration"
                );
            }
            return Ok(process);
        }
        entries.remove(&key);
    }

    let child = spawn()?;
    let process = Arc::new(ManagedProcess {
        key: key.clone(),
        signature,
        child: Mutex::new(Some(child)),
    });
    entries.insert(key, Arc::downgrade(&process));
    Ok(process)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::Command;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn unique_key(label: &str) -> String {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must be valid")
            .as_nanos();
        format!("{label}-{nonce}")
    }

    fn short_lived_process(key: String) -> Arc<ManagedProcess> {
        let process = spawn_managed(key, "mode=cpu", || {
            Command::new("sh")
                .args(["-c", "exit 0"])
                .spawn()
                .map_err(Into::into)
        })
        .expect("short-lived process must start");
        for _ in 0..100 {
            if !process.is_alive() {
                return process;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("short-lived process did not exit in time");
    }

    #[test]
    fn shared_lease_keeps_child_alive_until_last_owner_drops() {
        let key = unique_key("lease");
        let first = spawn_managed(key.clone(), "same", || {
            Command::new("sh")
                .args(["-c", "sleep 30"])
                .spawn()
                .map_err(Into::into)
        })
        .expect("test process must start");
        let pid = first.pid().expect("managed process must have a pid");
        let second = existing(&key, "same")
            .expect("registry lookup must work")
            .expect("managed process must be reusable");
        assert!(Arc::ptr_eq(&first, &second));

        drop(first);
        assert!(
            second.is_alive(),
            "dropping one engine lease must not kill the child"
        );
        drop(second);

        let status = Command::new("sh")
            .args(["-c", &format!("kill -0 {pid} 2>/dev/null")])
            .status()
            .expect("process existence check must run");
        assert!(!status.success(), "last lease must stop and reap the child");
    }

    #[test]
    fn conflicting_launch_configuration_is_rejected() {
        let key = unique_key("signature");
        let process = spawn_managed(key.clone(), "mode=cpu", || {
            Command::new("sh")
                .args(["-c", "sleep 30"])
                .spawn()
                .map_err(Into::into)
        })
        .expect("test process must start");

        let error = existing(&key, "mode=gpu").expect_err("conflicting config must fail");
        assert!(error.to_string().contains("different launch configuration"));
        drop(process);
    }

    #[test]
    fn dead_process_does_not_conflict_with_new_signature_lookup() {
        let key = unique_key("dead-existing");
        let dead = short_lived_process(key.clone());
        assert!(existing(&key, "mode=gpu")
            .expect("dead process must not create a signature conflict")
            .is_none());
        drop(dead);
    }

    #[test]
    fn dead_process_can_be_replaced_with_new_signature() {
        let key = unique_key("dead-spawn");
        let dead = short_lived_process(key.clone());
        let replacement = spawn_managed(key, "mode=gpu", || {
            Command::new("sh")
                .args(["-c", "sleep 30"])
                .spawn()
                .map_err(Into::into)
        })
        .expect("dead process must not block a replacement signature");
        assert!(replacement.is_alive());
        drop(dead);
        drop(replacement);
    }
}
