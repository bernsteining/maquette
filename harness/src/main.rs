use std::io::Write;
use std::time::Instant;
use wasmi::*;

thread_local! { static PROF_T0: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) }; }

struct HostState {
    args: Vec<u8>,
    result: Vec<u8>,
    prof: Vec<(i32, u64)>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let mut fuel = false;
    let mut bench: Option<usize> = None;
    let mut repeat: usize = 1;
    let mut positional = Vec::new();

    while let Some(arg) = args.next() {
        if arg == "--fuel" {
            fuel = true;
        } else if arg.starts_with("--bench=") {
            bench = Some(arg["--bench=".len()..].parse()?);
        } else if let Some(r) = arg.strip_prefix("--repeat=") {
            repeat = r.parse()?;
        } else if arg == "--bench" {
            bench = Some(args.next().ok_or("--bench requires a value")?.parse()?);
        } else {
            positional.push(arg);
        }
    }

    if positional.len() < 3 || positional.len() > 6 {
        eprintln!("usage: harness [--fuel] [--bench=N] <wasm> <func> <file1> [<file2>] [<file3>] [<file4>]");
        std::process::exit(1);
    }

    let wasm_path = &positional[0];
    let func_name = &positional[1];
    let file1 = std::fs::read(&positional[2])?;
    let file2: Option<Vec<u8>> = if positional.len() >= 4 {
        Some(std::fs::read(&positional[3])?)
    } else { None };
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
    let t_mod = Instant::now();
    let module = Module::new(&engine, &wasm_bytes)?;
    let t_mod = t_mod.elapsed();

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
                    if let Some(f2) = &file2 { a.extend_from_slice(f2); }
                    if let Some(f3) = &file3 { a.extend_from_slice(f3); }
                    if let Some(f4) = &file4 { a.extend_from_slice(f4); }
                    a
                },
                result: Vec::new(),
                prof: Vec::new(),
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

        linker.func_wrap(
            "typst_env",
            "maquette_prof",
            |mut caller: Caller<'_, HostState>, id: i32| {
                let v = match caller.get_fuel() {
                    Ok(f) => u64::MAX - f,
                    Err(_) => PROF_T0.with(|t| t.get().map(|t0| t0.elapsed().as_nanos() as u64).unwrap_or(0)),
                };
                caller.data_mut().prof.push((id, v));
            },
        )?;

        let instance = linker.instantiate_and_start(&mut store, &module)?;

        let func = instance
            .get_func(&store, func_name)
            .ok_or_else(|| format!("function '{}' not found", func_name))?;

        let mut params_v = vec![Val::I32(file1.len() as i32)];
        if let Some(f2) = &file2 { params_v.push(Val::I32(f2.len() as i32)); }
        if let Some(f3) = &file3 { params_v.push(Val::I32(f3.len() as i32)); }
        if let Some(f4) = &file4 { params_v.push(Val::I32(f4.len() as i32)); }
        let params = params_v;
        let mut results = [Val::I32(0)];

        for _ in 1..repeat {
            let a = store.data().args.clone();
            func.call(&mut store, &params, &mut results)?;
            store.data_mut().args = a;
            store.data_mut().prof.clear();
        }
        let start = Instant::now();
        PROF_T0.with(|t| t.set(Some(start)));
        let call_result = func.call(&mut store, &params, &mut results);
        let elapsed = start.elapsed();
        times.push(elapsed);

        if let Err(e) = call_result {
            eprintln!("[diag] trap: {:?}", e);
            match instance.get_func(&store, "get_last_panic") {
                Some(panic_fn) => {
                    eprintln!("[diag] found get_last_panic");
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

        if i + 1 == iterations && !store.data().prof.is_empty() {
            let marks = std::mem::take(&mut store.data_mut().prof);
            let mut prev = 0u64;
            let mut order: Vec<i32> = Vec::new();
            let mut agg: std::collections::HashMap<i32, (u64, u64, u64)> = Default::default();
            for (id, f) in marks {
                let e = agg.entry(id).or_insert_with(|| { order.push(id); (0, 0, 0) });
                e.0 += f - prev;
                e.1 += 1;
                e.2 = f;
                prev = f;
            }
            for id in order {
                let (sum, n, last) = agg[&id];
                eprintln!("prof: {:>4} {:>14} {:>14} n={}", id, sum, last, n);
            }
        }

        last_result = result_bytes;
    }

    std::io::stdout().write_all(&last_result)?;

    if bench.is_some() {
        let min = times.iter().min().unwrap();
        let avg = times.iter().sum::<std::time::Duration>() / times.len() as u32;
        eprintln!("module: {:.3?}", t_mod);
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
