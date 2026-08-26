use std::io::Write;
use std::time::Instant;
use wasmi::*;

struct HostState {
    args: Vec<u8>,
    result: Vec<u8>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut fuel = false;
    let mut bench: Option<usize> = None;
    let mut positional = Vec::new();

    while let Some(arg) = args.next() {
        if arg == "--fuel" {
            fuel = true;
        } else if arg.starts_with("--bench=") {
            bench = Some(arg["--bench=".len()..].parse()?);
        } else if arg == "--bench" {
            bench = Some(args.next().ok_or("--bench requires a value")?.parse()?);
        } else {
            positional.push(arg);
        }
    }

    if positional.len() < 4 || positional.len() > 6 {
        eprintln!("usage: harness [--fuel] [--bench=N] <wasm> <func> <file1> <file2> [<file3>] [<file4>]");
        std::process::exit(1);
    }

    let wasm_path = &positional[0];
    let func_name = &positional[1];
    let file1 = std::fs::read(&positional[2])?;
    let file2 = std::fs::read(&positional[3])?;
    let file3: Option<Vec<u8>> = if positional.len() >= 5 {
        Some(std::fs::read(&positional[4])?)
    } else { None };
    let file4: Option<Vec<u8>> = if positional.len() >= 6 {
        Some(std::fs::read(&positional[5])?)
    } else { None };

    let wasm_bytes = std::fs::read(wasm_path)?;

    let mut config = Config::default();
    if fuel {
        config.consume_fuel(true);
    }
    let engine = Engine::new(&config);
    let module = Module::new(&engine, &wasm_bytes)?;

    let iterations = bench.unwrap_or(1);
    let mut times = Vec::with_capacity(iterations);
    let mut fuel_used = 0u64;
    let mut last_result = Vec::new();

    for i in 0..iterations {
        let mut store = Store::new(
            &engine,
            HostState {
                args: {
                    let mut a: Vec<u8> = Vec::new();
                    a.extend_from_slice(&file1);
                    a.extend_from_slice(&file2);
                    if let Some(f3) = &file3 { a.extend_from_slice(f3); }
                    if let Some(f4) = &file4 { a.extend_from_slice(f4); }
                    a
                },
                result: Vec::new(),
            },
        );

        if fuel {
            store.set_fuel(u64::MAX)?;
        }

        let mut linker = <Linker<HostState>>::new(&engine);

        linker.func_wrap(
            "typst_env",
            "wasm_minimal_protocol_write_args_to_buffer",
            |mut caller: Caller<'_, HostState>, ptr: i32| {
                let args = caller.data().args.clone();
                let mem = caller.get_export("memory").unwrap().into_memory().unwrap();
                mem.write(&mut caller, ptr as usize, &args).unwrap();
            },
        )?;

        linker.func_wrap(
            "typst_env",
            "wasm_minimal_protocol_send_result_to_host",
            |mut caller: Caller<'_, HostState>, ptr: i32, len: i32| {
                let mem = caller.get_export("memory").unwrap().into_memory().unwrap();
                let mut buf = vec![0u8; len as usize];
                mem.read(&caller, ptr as usize, &mut buf).unwrap();
                caller.data_mut().result = buf;
            },
        )?;

        let instance = linker.instantiate_and_start(&mut store, &module)?;

        let func = instance
            .get_func(&store, func_name)
            .ok_or_else(|| format!("function '{}' not found", func_name))?;

        let mut params_v = vec![Val::I32(file1.len() as i32), Val::I32(file2.len() as i32)];
        if let Some(f3) = &file3 { params_v.push(Val::I32(f3.len() as i32)); }
        if let Some(f4) = &file4 { params_v.push(Val::I32(f4.len() as i32)); }
        let params = params_v;
        let mut results = [Val::I32(0)];

        let start = Instant::now();
        let call_result = func.call(&mut store, &params, &mut results);
        let elapsed = start.elapsed();
        times.push(elapsed);

        // If the wasm trapped, ask the plugin's panic hook for the message.
        if let Err(e) = call_result {
            eprintln!("[diag] trap: {:?}", e);
            match instance.get_func(&store, "get_last_panic") {
                Some(panic_fn) => {
                    eprintln!("[diag] found get_last_panic");
                    // wasm-minimal-protocol wraps every #[wasm_func] to return
                    // an i32 (0 = ok, negative = err), so we need a 1-slot
                    // results buffer even for a zero-arg getter.
                    let mut r = [Val::I32(0)];
                    store.data_mut().args = Vec::new();
                    match panic_fn.call(&mut store, &[], &mut r) {
                        Ok(()) => {
                            let msg = std::mem::take(&mut store.data_mut().result);
                            eprintln!("[diag] get_last_panic returned {} bytes", msg.len());
                            if !msg.is_empty() {
                                eprintln!("wasm panic captured: {}", String::from_utf8_lossy(&msg));
                            }
                        }
                        Err(pe) => eprintln!("[diag] get_last_panic call errored: {:?}", pe),
                    }
                }
                None => eprintln!("[diag] get_last_panic export not found"),
            }
            return Err(Box::new(e) as Box<dyn std::error::Error>);
        }

        let ret = results[0].i32().unwrap_or(-1);
        let result_bytes = std::mem::take(&mut store.data_mut().result);

        if fuel {
            let remaining = store.get_fuel()?;
            fuel_used = u64::MAX - remaining;
        }

        if ret != 0 {
            let msg = String::from_utf8_lossy(&result_bytes);
            eprintln!("error (iteration {}): {}", i + 1, msg);
            std::process::exit(1);
        }

        last_result = result_bytes;
    }

    // Write result to stdout
    std::io::stdout().write_all(&last_result)?;

    // Report to stderr
    if bench.is_some() {
        let min = times.iter().min().unwrap();
        let avg = times.iter().sum::<std::time::Duration>() / times.len() as u32;
        eprintln!("iterations: {}", iterations);
        eprintln!("avg: {:.3?}", avg);
        eprintln!("min: {:.3?}", min);
        if fuel {
            eprintln!("fuel: {}", fuel_used);
        }
    } else if fuel {
        eprintln!("fuel: {}", fuel_used);
    }

    Ok(())
}
