use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use vore_core::rpc::*;
use vore_core::rpc::{CommandCenter, Request};
use vore_core::{CloneableUnixStream, VirtualMachineInfo};

pub struct Client {
    stream: CloneableUnixStream,
    buf_reader: BufReader<CloneableUnixStream>,
    center: CommandCenter,
}

impl Client {
    pub fn connect<P: AsRef<Path>>(path: P) -> anyhow::Result<Client> {
        let path = path.as_ref();
        let stream = CloneableUnixStream::new(UnixStream::connect(path)?);
        log::debug!("Connected to vore socket at {}", path.to_str().unwrap());

        Ok(Client {
            buf_reader: BufReader::new(stream.clone()),
            stream,
            center: Default::default(),
        })
    }

    fn send<R: Request>(&mut self, request: R) -> anyhow::Result<R::Response> {
        let (_, json) = self.center.write_command(request)?;
        self.stream.write_all(json.as_bytes())?;
        let mut response = String::new();
        self.buf_reader.read_line(&mut response)?;
        let (_, info) = CommandCenter::read_answer::<R>(&response)?;
        Ok(info)
    }

    pub fn load_vm(
        &mut self,
        toml: &str,
        save: bool,
        cdroms: Vec<String>,
    ) -> anyhow::Result<VirtualMachineInfo> {
        Ok(self
            .send(LoadRequest {
                cdroms,
                save,
                toml: toml.to_string(),
                working_directory: None,
            })?
            .info)
    }

    pub fn list_vms(&mut self) -> anyhow::Result<Vec<VirtualMachineInfo>> {
        Ok(self.send(ListRequest {})?.items)
    }

    pub fn list_disk_presets(&mut self) -> anyhow::Result<Vec<DiskPreset>> {
        Ok(self.send(DiskPresetsRequest {})?.presets)
    }

    pub fn host_version(&mut self) -> anyhow::Result<InfoResponse> {
        self.send(InfoRequest {})
    }

    pub fn prepare(&mut self, vm: String, cdroms: Vec<String>) -> anyhow::Result<()> {
        self.send(PrepareRequest { name: vm, cdroms })?;
        Ok(())
    }

    pub fn start(&mut self, vm: String, cdroms: Vec<String>) -> anyhow::Result<()> {
        self.send(StartRequest { name: vm, cdroms })?;
        Ok(())
    }

    pub fn stop(&mut self, vm: String) -> anyhow::Result<()> {
        self.send(StopRequest { name: vm })?;
        Ok(())
    }
}
