const WASI_SNAPSHOT_PREVIEW1: &[u8] =
    include_bytes!("../../../wasmtime_wasi/wasi_snapshot_preview1.wasm");

pub fn componentize(wasm_bytecode: &[u8]) -> anyhow::Result<Vec<u8>> {
    wit_component::ComponentEncoder::default()
        .validate(true)
        .module(wasm_bytecode)?
        .adapter("wasi_snapshot_preview1", WASI_SNAPSHOT_PREVIEW1)?
        .encode()
}
