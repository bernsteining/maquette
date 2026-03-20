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

    if positional.len() != 4 {
        eprintln!("usage: harness [--fuel] [--bench=N] <wasm> <func> <file1> <file2>");
        std::process::exit(1);
    }

    let wasm_path = &positional[0];
    let func_name = &positional[1];
    let file1 = std::fs::read(&positional[2])?;
    let file2 = std::fs::read(&positional[3])?;

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
                args: [file1.as_slice(), file2.as_slice()].concat(),
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

        let params = [Val::I32(file1.len() as i32), Val::I32(file2.len() as i32)];
        let mut results = [Val::I32(0)];

        let start = Instant::now();
        func.call(&mut store, &params, &mut results)?;
        let elapsed = start.elapsed();
        times.push(elapsed);

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
