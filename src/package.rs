use std::fs::{File, Permissions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub trait Package {
    fn install(&self, dir: &Path);
}

pub struct SingleFilePackage {
    path: PathBuf,
    data: Box<[u8]>,
    perms: Permissions,
}

impl SingleFilePackage {
    pub fn new(path: PathBuf, data: Box<[u8]>, perms: Permissions)
        -> SingleFilePackage {
        SingleFilePackage { path, data, perms }
    }
}

impl Package for SingleFilePackage {
    fn install(&self, dir: &Path) {
        let mut file = File::create(dir.join(&self.path)).unwrap();
        file.set_permissions(self.perms.clone()).unwrap();
        file.write_all(&self.data).unwrap();
    }
}
