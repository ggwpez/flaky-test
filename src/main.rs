use anyhow::Result;
use clap::Parser;
use tokio::process;
use camino::Utf8PathBuf;
use rand_distr::Normal;
use rand_distr::Distribution;

#[derive(Clone, Debug, Parser)]
struct Opt {
    /// Path to the Cargo.toml file.
    #[clap(long, short)]
    manifest_path: Option<Utf8PathBuf>,

    /// Rust package name.
    #[clap(long, short)]
    package: String,

    /// Rust test name.
    #[clap(index = 1)]
    test: String,

    /// Timeout in seconds for each test run.
    #[clap(long, short, default_value = "60")]
    timeout: u64,

    #[clap(long, short, default_value = "30")]
    slow: u64,

    #[clap(long, short, default_value = "10")]
    batch: u64,

    #[clap(long, short, default_value = "10")]
    reps: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let opts = Opt::parse();

    let batch = opts.batch;
    let reps = opts.reps;
    let bin = opts.build(false).await?;

    // estimatedly measure one run:
    let now = std::time::Instant::now();
    opts.run_test(&bin, &opts.test, opts.timeout).await?;
    let took = now.elapsed();
    log::info!("Estimated time for one run: {:?}", took);

    // Run the binary 100 times, 10 times in parallel
    for rep in 0..reps {
        log::info!("Tests matrix {}x{}: {}/{}", batch, reps, rep + 1, reps);
        let mut futures = Vec::new();
        //let start_batch = std::time::Instant::now();

        // Sample a random value from a standard normal distribution centered on `1 / took`:
        let dist = Normal::new(took.as_secs_f64(), took.as_secs_f64() / 2.)?;
        for i in 0..batch {
            let opts = opts.clone();
            let bin = bin.clone();

            let task = tokio::spawn(async move {
                if i != 0 {
                    let duration = std::time::Duration::from_secs_f64(dist.sample(&mut rand::thread_rng()).abs().min(took.as_secs_f64() * 2.));
                    tokio::time::sleep(duration).await;
                    log::debug!("Slept for {:?}", duration);
                }
                opts.run_test(&bin, &opts.test, opts.timeout).await
            });
            futures.push(task);
        }

        eprintln!();
        let res = futures::future::try_join_all(futures).await?;
        for r in res.into_iter() {
            if let Err(e) = r {
                log::error!("Test failed: {:?}",e);
                return Err(e.into());
            }
        }

        //let elapsed = start_batch.elapsed();
        //log::debug!("Adjusting estimated time from {:?} to {:?}. Run took {:?} with batch size {}", took, elapsed, elapsed, batch);
       // took = elapsed;
    }

    Ok(())
}

impl Opt {
    async fn run_test(&self, bin: &Utf8PathBuf, test: &str, timeout: u64) -> Result<()> {
        let mut cargo = process::Command::new(bin);
        cargo.arg(test);
        cargo.kill_on_drop(true);
        cargo.stdout(std::process::Stdio::piped());
        cargo.stderr(std::process::Stdio::piped());

        log::trace!("Running {:?}", cargo);
        let now = std::time::Instant::now();
        let child = cargo.spawn()?;
        let (id, fut) = (child.id().expect("Need child PID"), child.wait_with_output());
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);

        // Spawn an additional thread to keep the child future alive even if we time out. A normal
        // `select` would drop the future in that case.
        tokio::spawn(async move {
            tokio::select! {
                res = fut => {
                    tx.send(res).await.unwrap();
                },
                _ = tx.closed() => {
                    log::warn!("Task with PID {:?} was killed", id);
                }
            }
        });

        let output = tokio::select! {
            _ = tokio::time::sleep(tokio::time::Duration::from_secs(timeout)) => {
                let path = dump_trace(id).await;
                log::error!("Task with PID {} is too slow trace stored to {}, you meay want to attach a attach a debugger", path, id);
                tokio::time::sleep(tokio::time::Duration::from_secs(1<<10)).await;
                anyhow::bail!("Test timed out")
            },
            Some(output) = rx.recv() => {
                let output = output?;
                if !output.status.success() {
                    anyhow::bail!("Test failed: {:?}", output);
                }
                output
            }
        };

        let stdout = String::from_utf8(output.stdout)?;
        stdout
            .lines()
            .find(|line| line.contains("running 1 test"))
            .ok_or_else(|| anyhow::anyhow!("Test did not run. Wrong name?"))?;

        if now.elapsed().as_secs() > self.slow {
            log::warn!("Test took {:?}", now.elapsed());
        } else {
            log::debug!("Test took {:?}", now.elapsed());
        }

        Ok(())
    }

    async fn build(&self, release: bool) -> Result<Utf8PathBuf> {
        let mut cargo = self.build_command(release);

        log::info!("Running {:?}", cargo);
        let output = cargo.output().await?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("Failed to build"));
        }

        let stdout = String::from_utf8(output.stderr)?;
        let bin_name = stdout
            .lines()
            .find(|line| line.contains("Executable "))
            .map(|line| line.split_whitespace().last())
            .flatten()
            .ok_or_else(|| anyhow::anyhow!("Failed to find executable name"))?;
        let bin_name = bin_name.replace("(", "").replace(")", "");
        log::info!("Found binary: {}", bin_name);
        
        Ok(Utf8PathBuf::from(bin_name))
    }

    fn build_command(&self, release: bool) -> process::Command {
        let mut cargo = process::Command::new("cargo");
        cargo.kill_on_drop(true);
        cargo.env("RUSTFLAGS", "-Cdebug-assertions=y -g"); // assertions + debug symbols
        cargo.arg("test");
        cargo.arg("--package");
        cargo.arg(&self.package);
        cargo.arg("--no-run");

        if release {
            cargo.arg("--release");
        }
        if let Some(mut manifest_path) = self.manifest_path.clone() {
            // lets make our life a bit easier.
            if !manifest_path.ends_with("Cargo.toml") {
                manifest_path.push("Cargo.toml");
            }
            cargo.arg("--manifest-path");
            cargo.arg(manifest_path);
        }

        cargo
    }
}

async fn dump_trace(pid: u32) -> Utf8PathBuf {
    let path = format!("trace_{}.txt", pid);
    // run lldb
    let mut lldb = process::Command::new("lldb");
    lldb.kill_on_drop(true);
    lldb.arg("attach");
    lldb.arg("-p");
    lldb.arg(pid.to_string());
    lldb.arg("-o");
    lldb.arg("thread backtrace all");
    lldb.arg("-o");
    lldb.arg("quit");

    let output = lldb.output().await.unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("thread #"), "sanity check failed");
    std::fs::write(&path, stdout).unwrap();
    Utf8PathBuf::from(path)
}
