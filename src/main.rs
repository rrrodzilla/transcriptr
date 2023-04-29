use clap::{arg, command, value_parser, ValueEnum};
use serde_json::{Deserializer, Value};
use std::collections::HashMap;
use std::error::Error;
use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use time::Duration;

// Define an enum for the output format, which can be Text, Html, or Json
#[derive(ValueEnum, Copy, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum OutputFormat {
    Text,
    Html,
    Json,
}

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::init(); // Initialize the logger

    // Define the command line arguments using clap
    let matches = command!()
        .arg_required_else_help(true)
        .arg(
            arg!(-i --input <FILE> "Sets the input file to transcribe")
                .required(true)
        )
        .arg(
             arg!(-o --output <FILE> "Sets the output file to write the transcription to")
                .default_value("-")
                .required(false)
        )
        .arg(
             arg!(-f --format <OutputFormat> "Sets the output format to text, html, or json")
                .default_value("text")
                .value_parser(value_parser!(OutputFormat))
                .required(false),
        )
        .arg(
             arg!(-s --speaker_format <FORMAT> "Sets the format for the speaker labels in the output")
                .default_value("{}:")
                .required(false),
        )
        .arg(
             arg!(-l --line_break <LINE_BREAK> "Sets the line break behavior for the output. Can be 'auto' or 'manual'")
                .default_value("auto")
                .required(false),
        )
        .get_matches();

    // Retrieve the command line arguments
    let speaker_format: &String = matches
        .get_one("speaker_format")
        .ok_or("Couldn't find speaker-format arg value")?;
    let line_break: &String = matches
        .get_one("line_break")
        .ok_or("Couldn't find line-break arg value")?;
    let input_path: &String = matches
        .get_one("input")
        .ok_or("Please specify the input file.")?;
    let output_path: &String = matches
        .get_one("output")
        .ok_or("Please provide an output path.")?;

    // Determine the output format
    let output_format = matches
        .get_one::<OutputFormat>("format")
        .ok_or("Please provide an output format.")?;

    // Open the output file, which can be stdout if the output argument is "-"
    let output_file = if output_path == "-" {
        Box::new(std::io::stdout()) as Box<dyn Write>
    } else {
        Box::new(
            File::create(output_path)
                .map_err(|e| format!("Error creating the output file: {}.", e))?,
        )
    };

    let mut writer = BufWriter::new(output_file);

    // Open the input file, which can be stdin if the input argument is "-"
    let input_reader: Box<dyn Read> = if input_path == "-" {
        Box::new(std::io::stdin())
    } else {
        Box::new(
            std::fs::File::open(input_path)
                .map_err(|e| format!("Error opening the input file: {}.", e))?,
        )
    };
    let reader = BufReader::new(input_reader);
    let stream = Deserializer::from_reader(reader).into_iter::<Value>(); // Create a JSON deserializer for the
                                                                         // Create a HashMap to store speaker start times
    let mut speaker_start_times = HashMap::new();
    // Create a vector to store transcription items
    let mut items = Vec::new();

    // Loop over the JSON values in the input file
    for value in stream {
        if let Err(error) = value {
            match error.classify() {
                serde_json::error::Category::Syntax => {
                    eprintln!("An error was caused by data that was not syntactically valid JSON. Line {}: column {}", error.line(), error.column())
                }
                serde_json::error::Category::Io => eprintln!(
                    "An error was caused by failure to read or write bytes to an IO stream."
                ),
                serde_json::error::Category::Eof => eprintln!(
                    "An error was caused by prematurely reaching the end of the input data."
                ),
                serde_json::error::Category::Data => {
                    eprintln!("An error was caused by input data that was semantically incorrect.")
                }
            }
            std::process::exit(1);
        }
        let value = value?;
        // Check if the value contains speaker labels
        if let Some(labels) = value
            .pointer("/results/speaker_labels/segments")
            .and_then(Value::as_array)
        {
            // Loop over each label and its items
            for label in labels {
                for item in label["items"].as_array().unwrap() {
                    // Get the start time and speaker label for each item, and add it to the HashMap
                    let start_time = item["start_time"]
                        .as_str()
                        .ok_or("Error: Start time is missing in the speaker label item.")?
                        .to_string();

                    let speaker_label = item["speaker_label"]
                        .as_str()
                        .ok_or("Error: Speaker label is missing in the speaker label item.")?
                        .to_string();
                    speaker_start_times.insert(start_time, speaker_label);
                }
            }
        }

        // Check if the value contains transcription items
        if let Some(new_items) = value.pointer("/results/items").and_then(Value::as_array) {
            // Add the new items to the items vector
            items.extend_from_slice(new_items);
        }
    }

    // Create a vector to store lines of transcription
    let mut lines = Vec::new();
    // Create an empty string to store each line
    let mut line = String::new();
    // Initialize the time of each line
    let mut time = 0.0;
    // Initialize the speaker of each line as "null"
    let mut speaker = "null".to_string();
    // Initialize the current speaker as the same as the speaker of the previous line
    let mut current_speaker = speaker.clone();

    // Loop over each transcription item and construct lines of transcription
    items
        .into_iter()
        .try_for_each(|item| -> Result<(), Box<dyn Error>> {
            let content = item["alternatives"][0]["content"].as_str().unwrap(); // Get the content of the item

            if let Some(start_time_str) = item.get("start_time").and_then(Value::as_str) {
                // If the item has a start time, update the current speaker and time
                current_speaker = speaker_start_times.get(start_time_str).unwrap().clone();
                time = start_time_str.parse().unwrap_or(0.0);
            } else if item["type"] == "punctuation" {
                // If the item is punctuation, add the punctuation to the current line
                line.push_str(content);
            }

            // Check if the speaker has changed
            if current_speaker != speaker {
                // If the speaker has changed, push the previous line to the lines vector and start a new line
                if !speaker.is_empty() {
                    lines.push((time, speaker.clone(), line.clone()));
                }
                line = content.to_string();
                speaker = current_speaker.clone();
            } else if item["type"] != "punctuation" {
                // If the speaker has not changed and the item is not punctuation, add a space and the content to the current line
                line.push(' ');
                line.push_str(content);
            }

            Ok(())
        })
        .map_err(|e| format!("Error processing transcription items: {:?}.", e))?;

    // Push the final line to the lines vector
    lines.push((time, speaker, line));

    // Sort the lines

    lines.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());

    // Write the output based on the chosen format
    match output_format {
        OutputFormat::Text => {
            // Loop over the lines and format them as text
            lines
                .into_iter()
                .try_for_each(|(time, speaker, line)| -> Result<(), Box<dyn Error>> {
                    let duration = Duration::seconds_f64(time);

                    let speaker_str = speaker_format.replace("{}", &speaker);

                    let formatted_line = format!(
                        "[{:02}:{:02}:{:02}] {} {}{}",
                        duration.whole_hours(),
                        duration.whole_minutes() % 60,
                        duration.whole_seconds() % 60,
                        speaker_str,
                        line,
                        if line_break == "auto" { "\n" } else { "" }
                    );
                    writeln!(&mut writer, "{}", formatted_line)?;
                    Ok(())
                })
                .map_err(|e| format!("Error writing lines to the output file: {:?}.", e))?;
        }
        OutputFormat::Html => {
            // Write the HTML boilerplate
            writeln!(
            &mut writer,
            "<!DOCTYPE html>\n<html>\n<head>\n<title>Transcription</title>\n<style>\n.speaker {{ font-weight: bold; }}\n</style>\n</head>\n<body>"
        )?;

            // Loop over the lines and format them as HTML
            lines
                .into_iter()
                .try_for_each(|(time, speaker, line)| -> Result<(), Box<dyn Error>> {
                    let duration = Duration::seconds_f64(time);
                    let speaker_str = speaker_format.replace("{}", &speaker);
                    let formatted_line = format!(
                        "[{:02}:{:02}:{:02}] <span class=\"speaker\">{}</span>: {}<br>",
                        duration.whole_hours(),
                        duration.whole_minutes() % 60,
                        duration.whole_seconds() % 60,
                        speaker_str,
                        line
                    );
                    writeln!(&mut writer, "{}", formatted_line)?;
                    Ok(())
                })
                .map_err(|e| format!("Error writing lines to the output file: {:?}.", e))?;

            // Write the closing HTML tags
            writeln!(&mut writer, "</body>\n</html>")?;
        }
        OutputFormat::Json => {
            // Construct a JSON object with the transcription data
            let output_data = serde_json::json!({
                "transcription": lines.iter().map(|(time, speaker, line)| {
                    let duration = Duration::seconds((time as &f64).round() as i64);
                    serde_json::json!({
                        "time": format!("{:02}:{:02}:{:02}", duration.whole_hours(), duration.whole_minutes() % 60, duration.whole_seconds() % 60),
                        "speaker": speaker,
                        "line": line,
                    })
                }).collect::<Vec<_>>(),
            });

            // Write the JSON object to the output file
            serde_json::to_writer_pretty(&mut writer, &output_data)?;
        }
    }

    // Add a log event to show the program completed successfully
    log::info!("Program completed successfully.");

    Ok(())
}
