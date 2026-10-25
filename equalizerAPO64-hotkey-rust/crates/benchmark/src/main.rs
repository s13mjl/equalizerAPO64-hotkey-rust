//! Port of the Equalizer APO `Benchmark` console application.
//!
//! Benchmark generates a linear sine sweep or reads a WAV file, filters it with
//! the Equalizer APO configuration via `eapo_engine::FilterEngine` and writes a
//! 16 bit WAV file to the requested path or to `%TEMP%\testout.wav`.
//!
//! The original used TCLAP for option parsing; this port keeps the same option
//! names, defaults and error behaviour with a small hand-written parser.

use std::path::PathBuf;

use eapo_common::log_helper;
use eapo_common::precision_timer::PrecisionTimer;
use eapo_engine::FilterEngine;

const VERSION: &str = env!("CARGO_PKG_VERSION");

struct Options {
    no_pause: bool,
    verbose: bool,
    guid: String,
    connection_name: String,
    device_name: String,
    batch_size: u32,
    output: String,
    input: String,
    rate: u32,
    to: f32,
    from: f32,
    length: f32,
    channels: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            no_pause: false,
            verbose: false,
            guid: String::new(),
            connection_name: "File output".to_string(),
            device_name: "Benchmark".to_string(),
            batch_size: 65536,
            output: String::new(),
            input: String::new(),
            rate: 44100,
            to: 20000.0,
            from: 1.0,
            length: 200.0,
            channels: 2,
        }
    }
}

struct ArgError {
    message: String,
    arg_id: String,
}

impl ArgError {
    fn new(message: &str, arg_id: &str) -> Self {
        Self {
            message: message.to_string(),
            arg_id: arg_id.to_string(),
        }
    }
}

#[derive(Default)]
struct Seen {
    no_pause: bool,
    verbose: bool,
    guid: bool,
    connection_name: bool,
    device_name: bool,
    batch_size: bool,
    output: bool,
    input: bool,
    rate: bool,
    to: bool,
    from: bool,
    length: bool,
    channels: bool,
}

fn print_help() {
    println!(
        "Benchmark generates a linear sine sweep or reads from the given input file. It then filters the waveform using the Equalizer APO filter configuration and finally writes to the given file or into the user's temp directory."
    );
    println!();
    println!("Usage: Benchmark [options]");
    println!();
    println!("Options:");
    println!("     --nopause                           Do not wait for key press at the end");
    println!("  -v --verbose                           Print trace and error messages to console instead of logfile");
    println!("     --guid <string>                     Endpoint GUID to use when parsing configuration (Default: <empty>)");
    println!("     --connectionname <string>           Connection name to use when parsing configuration (Default: File output)");
    println!("     --devicename <string>               Device name to use when parsing configuration (Default: Benchmark)");
    println!("     --batchsize <integer>               Number of frames processed in one batch (Default: 65536)");
    println!("  -o --output <string>                    File to write sound data to");
    println!("  -i --input <string>                     File to load sound data from instead of generating sweep");
    println!("  -r --rate <integer>                     Sample rate of generated sweep (Default: 44100)");
    println!("  -t --to <float>                         End frequency of generated sweep in Hz (Default: 20000.0)");
    println!("  -f --from <float>                       Start frequency of generated sweep in Hz (Default: 0.1)");
    println!("  -l --length <float>                     Length of generated sweep in seconds (Default: 200.0)");
    println!("  -c --channels <integer>                 Number of channels of generated sweep (Default: 2)");
    println!("     --version                           Displays version information and exits.");
    println!("  -h --help                              Displays usage information and exits.");
}

fn parse_args(args: &[String]) -> Result<Options, ArgError> {
    let mut options = Options::default();
    let mut seen = Seen::default();
    let mut index = 0;

    let value_of = |index: &mut usize, inline: Option<String>, arg_id: &str| -> Result<String, ArgError> {
        if let Some(value) = inline {
            return Ok(value);
        }
        *index += 1;
        args.get(*index)
            .cloned()
            .ok_or_else(|| ArgError::new("Argument is missing", arg_id))
    };

    let mut end_of_options = false;
    while index < args.len() {
        let arg = &args[index];
        if end_of_options {
            return Err(ArgError::new("Unknown argument", arg));
        }
        if arg == "--" {
            end_of_options = true;
            index += 1;
            continue;
        }

        let (name, inline) = if let Some(long) = arg.strip_prefix("--") {
            match long.split_once('=') {
                Some((name, value)) => (format!("--{}", name), Some(value.to_string())),
                None => (arg.clone(), None),
            }
        } else if arg.starts_with('-') && arg.len() > 2 {
            // TCLAP accepts "-r44100" as well as "-r 44100".
            (arg[..2].to_string(), Some(arg[2..].to_string()))
        } else {
            (arg.clone(), None)
        };

        match name.as_str() {
            "-h" | "--help" => {
                print_help();
                std::process::exit(0);
            }
            "--version" => {
                println!("{}", VERSION);
                std::process::exit(0);
            }
            "-v" | "--verbose" => {
                if seen.verbose {
                    return Err(ArgError::new("Argument already set!", "-v"));
                }
                seen.verbose = true;
                options.verbose = true;
            }
            "--nopause" => {
                if seen.no_pause {
                    return Err(ArgError::new("Argument already set!", "--nopause"));
                }
                seen.no_pause = true;
                options.no_pause = true;
            }
            "--guid" => {
                if seen.guid {
                    return Err(ArgError::new("Argument already set!", "--guid"));
                }
                seen.guid = true;
                options.guid = value_of(&mut index, inline, "--guid")?;
            }
            "--connectionname" => {
                if seen.connection_name {
                    return Err(ArgError::new("Argument already set!", "--connectionname"));
                }
                seen.connection_name = true;
                options.connection_name = value_of(&mut index, inline, "--connectionname")?;
            }
            "--devicename" => {
                if seen.device_name {
                    return Err(ArgError::new("Argument already set!", "--devicename"));
                }
                seen.device_name = true;
                options.device_name = value_of(&mut index, inline, "--devicename")?;
            }
            "--batchsize" => {
                if seen.batch_size {
                    return Err(ArgError::new("Argument already set!", "--batchsize"));
                }
                seen.batch_size = true;
                let value = value_of(&mut index, inline, "--batchsize")?;
                options.batch_size = parse_u32(&value, "--batchsize")?;
            }
            "-o" | "--output" => {
                if seen.output {
                    return Err(ArgError::new("Argument already set!", "-o"));
                }
                seen.output = true;
                options.output = value_of(&mut index, inline, "-o")?;
            }
            "-i" | "--input" => {
                if seen.input {
                    return Err(ArgError::new("Argument already set!", "-i"));
                }
                seen.input = true;
                options.input = value_of(&mut index, inline, "-i")?;
            }
            "-r" | "--rate" => {
                if seen.rate {
                    return Err(ArgError::new("Argument already set!", "-r"));
                }
                seen.rate = true;
                let value = value_of(&mut index, inline, "-r")?;
                options.rate = parse_u32(&value, "-r")?;
            }
            "-t" | "--to" => {
                if seen.to {
                    return Err(ArgError::new("Argument already set!", "-t"));
                }
                seen.to = true;
                let value = value_of(&mut index, inline, "-t")?;
                options.to = parse_f32(&value, "-t")?;
            }
            "-f" | "--from" => {
                if seen.from {
                    return Err(ArgError::new("Argument already set!", "-f"));
                }
                seen.from = true;
                let value = value_of(&mut index, inline, "-f")?;
                options.from = parse_f32(&value, "-f")?;
            }
            "-l" | "--length" => {
                if seen.length {
                    return Err(ArgError::new("Argument already set!", "-l"));
                }
                seen.length = true;
                let value = value_of(&mut index, inline, "-l")?;
                options.length = parse_f32(&value, "-l")?;
            }
            "-c" | "--channels" => {
                if seen.channels {
                    return Err(ArgError::new("Argument already set!", "-c"));
                }
                seen.channels = true;
                let value = value_of(&mut index, inline, "-c")?;
                options.channels = parse_u32(&value, "-c")?;
            }
            other => {
                return Err(ArgError::new("Unknown argument", other));
            }
        }
        index += 1;
    }

    Ok(options)
}

fn parse_u32(value: &str, arg_id: &str) -> Result<u32, ArgError> {
    value.parse::<u32>().map_err(|_| {
        ArgError::new(
            &format!("Couldn't read argument value from string '{}'", value),
            arg_id,
        )
    })
}

fn parse_f32(value: &str, arg_id: &str) -> Result<f32, ArgError> {
    value.parse::<f32>().map_err(|_| {
        ArgError::new(
            &format!("Couldn't read argument value from string '{}'", value),
            arg_id,
        )
    })
}

/// C `%g` with the default precision of 6 significant digits.
fn format_g(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    if !value.is_finite() {
        return format!("{}", value);
    }
    let exponent = value.abs().log10().floor() as i32;
    if exponent < -4 || exponent >= 6 {
        let text = format!("{:.*e}", 5, value);
        // Rust prints "1.23456e3"; C prints "1.23456e+03".
        if let Some((mantissa, exp)) = text.split_once('e') {
            let exp_value: i32 = exp.parse().unwrap_or(0);
            let mut mantissa = mantissa.to_string();
            if mantissa.contains('.') {
                while mantissa.ends_with('0') {
                    mantissa.pop();
                }
                if mantissa.ends_with('.') {
                    mantissa.pop();
                }
            }
            format!(
                "{}e{}{:02}",
                mantissa,
                if exp_value < 0 { '-' } else { '+' },
                exp_value.abs()
            )
        } else {
            text
        }
    } else {
        let precision = (6 - 1 - exponent).max(0) as usize;
        let mut text = format!("{:.*}", precision, value);
        if text.contains('.') {
            while text.ends_with('0') {
                text.pop();
            }
            if text.ends_with('.') {
                text.pop();
            }
        }
        text
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let options = match parse_args(&args) {
        Ok(options) => options,
        Err(error) => {
            println!("Error: {} for arg {}", error.message, error.arg_id);
            std::process::exit(-1);
        }
    };

    let verbose = options.verbose;
    log_helper::set(verbose, true, true);

    println!("Benchmark {}", VERSION);
    let program_name = std::env::args().next().unwrap_or_else(|| "Benchmark".to_string());
    println!("Run \"{} -h\" to show usage info", program_name);
    println!();

    let sample_rate: u32;
    let channel_count: u32;
    let channel_mask: u32 = 0;
    let frame_count: u32;
    let length: f32;
    let mut input_buf: Vec<f32>;

    if !options.input.is_empty() {
        println!("Reading sound data from {}", options.input);

        let mut timer = PrecisionTimer::new();
        timer.start();

        let audio = match eapo_filters::audio_file::read_audio_file(&options.input) {
            Ok(audio) => audio,
            Err(error) => {
                eprintln!("{}", error);
                std::process::exit(1);
            }
        };
        if audio.channels.is_empty() {
            eprintln!("file has no audio channels");
            std::process::exit(1);
        }
        sample_rate = audio.sample_rate;
        channel_count = audio.channel_count() as u32;
        frame_count = audio.frames() as u32;
        length = frame_count as f32 / sample_rate as f32;

        input_buf = vec![0.0f32; frame_count as usize * channel_count as usize];
        for frame in 0..frame_count as usize {
            for channel in 0..channel_count as usize {
                input_buf[frame * channel_count as usize + channel] =
                    audio.channels[channel][frame] as f32;
            }
        }

        let read_time = timer.stop();
        println!("Reading input file took {} seconds", format_g(read_time));
    } else {
        sample_rate = options.rate;
        channel_count = options.channels;
        let sweep_from = options.from;
        let sweep_to = options.to;
        let sweep_diff = sweep_to - sweep_from;
        length = options.length;
        frame_count = (length * sample_rate as f32) as u32;

        println!(
            "No input file given, so generating linear sine sweep from {} to {} Hz over {} seconds",
            format_g(sweep_from as f64),
            format_g(sweep_to as f64),
            format_g(length as f64)
        );

        let mut timer = PrecisionTimer::new();
        timer.start();

        input_buf = vec![0.0f32; frame_count as usize * channel_count as usize];
        for i in 0..frame_count {
            let t = i as f64 / sample_rate as f64;
            let s = (((sweep_from as f64
                + sweep_diff as f64 * (t / length as f64) / 2.0)
                * t)
                * 2.0
                * std::f64::consts::PI)
                .sin() as f32;
            for j in 0..channel_count {
                input_buf[(i * channel_count + j) as usize] = s;
            }
        }

        let gen_time = timer.stop();
        println!("Generating sweep took {} seconds", format_g(gen_time));
    }

    let batch_size = options.batch_size;
    let mut output_buf = vec![0.0f32; frame_count as usize * channel_count as usize];

    let mut timer = PrecisionTimer::new();
    timer.start();
    {
        let mut engine = FilterEngine::new();
        let device_name = options.device_name.clone();
        let connection_name = options.connection_name.clone();
        let device_guid = options.guid.clone();
        engine.set_device_info(
            false,
            true,
            &device_name,
            &connection_name,
            &device_guid,
            &format!("{} {} {}", device_name, connection_name, device_guid),
        );
        engine.initialize(
            sample_rate as f32,
            channel_count,
            channel_count,
            channel_count,
            channel_mask,
            batch_size,
            "",
        );

        let init_time = timer.stop();
        if !verbose {
            println!("\nLoading configuration took {} ms", format_g(init_time * 1000.0));
        }

        println!(
            "\nProcessing {} frames from {} channel(s)",
            frame_count, channel_count
        );

        timer.start();

        let mut processed = 0u32;
        while processed < frame_count {
            let frames = batch_size.min(frame_count - processed);
            unsafe {
                engine.process_f32_interleaved(
                    output_buf.as_mut_ptr().add((processed * channel_count) as usize),
                    input_buf.as_ptr().add((processed * channel_count) as usize),
                    frames,
                );
            }
            processed += frames;
        }

        let time = timer.stop();

        println!(
            "{} samples processed in {} seconds",
            frame_count as u64 * channel_count as u64,
            format!("{:.6}", time)
        );
        println!(
            "This is equivalent to {:.2}% CPU load (one core) when processing in real time",
            100.0 * time / length as f64
        );

        let mut clip_count = 0u32;
        let mut max = 0.0f32;
        for value in &output_buf {
            let value = value.abs();
            if value > max {
                max = value;
            }
            if value > 1.0f32 {
                clip_count += 1;
            }
        }

        print!(
            "Max output level: {:.6} ({:.6} dB)",
            max,
            (max as f64).log10() * 20.0
        );
        if clip_count > 0 {
            print!(" ({} samples clipped!)", clip_count);
        }
        println!();

        let output = if options.output.is_empty() {
            let mut path = PathBuf::from(std::env::temp_dir());
            path.push("testout.wav");
            path.to_string_lossy().to_string()
        } else {
            options.output.clone()
        };

        println!("\nWriting output to {}", output);

        let spec = hound::WavSpec {
            channels: channel_count as u16,
            sample_rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = match hound::WavWriter::create(&output, spec) {
            Ok(writer) => writer,
            Err(error) => {
                eprintln!("{}", error);
                std::process::exit(1);
            }
        };
        for value in &output_buf {
            // libsndfile writes normalized floats to PCM16 with clipping
            // disabled by default: scale by 32768, round (rintf), then let
            // the value wrap into the 16-bit range.
            let sample = ((*value as f64) * 32768.0).round_ties_even() as i64 as i16;
            if let Err(error) = writer.write_sample(sample) {
                eprintln!("{}", error);
                std::process::exit(1);
            }
        }
        if let Err(error) = writer.finalize() {
            eprintln!("{}", error);
            std::process::exit(1);
        }
    }

    if !options.no_pause {
        // `system("pause")` in the original: localized prompt, any key.
        let _ = std::process::Command::new("cmd").args(["/C", "pause"]).status();
    }
}
