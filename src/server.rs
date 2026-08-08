pub struct StartOptions {
    pub workspace_folders: Box<dyn Fn() -> Vec<String> + Send + Sync>,
    pub reveal: Box<dyn Fn(&str) + Send + Sync>,
    pub auth_token: Option<String>,
    pub port: Option<u16>,
}

pub struct Sidecar {
    _auth_token: String,
}

impl Sidecar {
    pub fn port(&self) -> u16 {
        todo!()
    }

    pub fn hostname(&self) -> &str {
        "127.0.0.1"
    }

    pub fn auth_token(&self) -> &str {
        &self._auth_token
    }

    pub fn set_focus(&self, _file_path: Option<&str>) {
        todo!()
    }

    pub fn focused_file(&self) -> Option<String> {
        todo!()
    }

    pub fn mention(&self, _file_paths: &[String]) {
        todo!()
    }

    pub fn stop(&self) {
        todo!()
    }
}

pub async fn start_sidecar(_opts: StartOptions) -> std::io::Result<Sidecar> {
    todo!()
}
