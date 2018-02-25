use std::io::{self, Read, Write};
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::{Once, ONCE_INIT};
use nix::unistd::Pid;
use rand::{self, Rng};

const CPUACCT_ROOT: &str = "/sys/fs/cgroup/cpuacct/sandbox";
const MEMORY_ROOT: &str = "/sys/fs/cgroup/memory/sandbox";
const PIDS_ROOT: &str = "/sys/fs/cgroup/pids/sandbox";
const CGROUP_ROOTS: [&str; 3] = [CPUACCT_ROOT, MEMORY_ROOT, PIDS_ROOT];
const CGROUP_NAME_LEN: usize = 16;

pub struct CGroup {
    cpuacct_dir: CGroupDir,
    memory_dir: CGroupDir,
    pids_dir: CGroupDir,
}

struct CGroupDir(PathBuf);

impl CGroup {
    pub fn new() -> CGroup {
        static INIT_CGROUP: Once = ONCE_INIT;
        INIT_CGROUP.call_once(|| {
            for &root in &CGROUP_ROOTS {
                let path = Path::new(root);
                if !path.is_dir() {
                    fs::create_dir_all(path).unwrap();
                }
            }
        });
        let cpuacct_dir = CGroupDir::new_in(Path::new(CPUACCT_ROOT)).unwrap();
        let memory_dir = CGroupDir::new_in(Path::new(MEMORY_ROOT)).unwrap();
        let pids_dir = CGroupDir::new_in(Path::new(PIDS_ROOT)).unwrap();
        CGroup { cpuacct_dir, memory_dir, pids_dir }
    }

    pub fn add_task(&mut self, pid: Pid) -> io::Result<()> {
        self.cpuacct_dir.write("tasks", &format!("{}", pid))?;
        self.memory_dir.write("tasks", &format!("{}", pid))?;
        self.pids_dir.write("tasks", &format!("{}", pid))?;
        Ok(())
    }

    pub fn procs(&self) -> io::Result<Vec<Pid>> {
        let mut pids = Vec::new();
        for &dir in &[&self.cpuacct_dir, &self.memory_dir, &self.pids_dir] {
            for line in dir.read("cgroup.procs")?.lines() {
                let pid = line.parse()
                    .map_err(|_| io::Error::from(io::ErrorKind::InvalidData))?;
                pids.push(Pid::from_raw(pid));
            }
        }
        pids.dedup();
        Ok(pids)
    }
}

impl CGroupDir {
    fn new_in(root_dir: &Path) -> io::Result<CGroupDir> {
        let mut rng = rand::thread_rng();
        loop {
            let name: String =
                rng.gen_ascii_chars().take(CGROUP_NAME_LEN).collect();
            let path = root_dir.join(name);
            match fs::create_dir(&path) {
                Ok(()) => return Ok(CGroupDir(path)),
                Err(ref e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                Err(e) => return Err(e),
            }
        }
    }

    fn read(&self, name: &str) -> io::Result<String> {
        let mut file = File::open(self.0.join(name))?;
        let mut result = String::new();
        file.read_to_string(&mut result)?;
        Ok(result)
    }

    fn write(&mut self, name: &str, data: &str) -> io::Result<()> {
        let mut file = File::create(self.0.join(name))?;
        file.write_all(data.as_bytes())?;
        Ok(())
    }
}

impl Drop for CGroupDir {
    fn drop(&mut self) {
        fs::remove_dir(&self.0).unwrap();
    }
}
