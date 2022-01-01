extern crate ffmpeg_next as ffmpeg;

use bincode;
use bliss_audio::distance::cosine_distance;
use bliss_audio::{Analysis, AnalysisIndex, Song};
use clap::{App, Arg, SubCommand};
use half::f16;
use rust_fuzzy_search::fuzzy_compare;
use serde::{Deserialize, Serialize};
use serde_json;
use sled::{self, Db};
use walkdir::{self, WalkDir};

use std::path::Path;
use std::process::Command;
use std::{env, error::Error};
use std::{fs, io};

#[derive(Serialize, Deserialize, Debug)]
struct Stored {
    fhash: [u8; 32],
    phash: u128,

    title: String,
    artist: String,
    album: String,

    analysis: Analysis,
}

fn main() -> Result<(), Box<dyn Error>> {
    let matches = App::new("wusic")
        .version(env!("CARGO_PKG_VERSION"))
        .author("wozeparrot")
        .about("A music storage system")
        .subcommand(
            SubCommand::with_name("ingest")
                .about("Ingest music into the database.")
                .arg(
                    Arg::with_name("path")
                        .long("path")
                        .help("Path to folder to ingest.")
                        .takes_value(true)
                        .required(true),
                )
                .arg(
                    Arg::with_name("copy")
                        .long("copy")
                        .help("Copy songs instead of transcoding (Make sure they are in the correct format first (Opus 160k 48k))")
                        .takes_value(false),
                ),
        )
        .subcommand(
            SubCommand::with_name("list")
                .about("List music in the database")
                .arg(
                    Arg::with_name("detailed")
                        .long("detailed")
                        .help("List song analysis information.")
                        .takes_value(false),
                ),
        )
        .subcommand(SubCommand::with_name("sync").about("Syncs database with the store"))
        .arg(
            Arg::with_name("db")
                .long("db")
                .help("Path to database.")
                .takes_value(true)
                .required(true),
        )
        .arg(
            Arg::with_name("store")
                .long("store")
                .help("Path to music store.")
                .takes_value(true)
                .required(true),
        )
        .get_matches();

    // Open database
    let db = sled::open(matches.value_of("db").unwrap())?;

    // Create music store directory
    let msp = Path::new(matches.value_of("store").unwrap());
    fs::create_dir_all(msp)?;

    ffmpeg::init().unwrap();

    // Handle subcommands
    if let Some(sub_m) = matches.subcommand_matches("ingest") {
        WalkDir::new(sub_m.value_of("path").unwrap())
            .into_iter()
            .filter_map(|file| file.ok())
            .filter(|file| file.metadata().unwrap().is_file())
            .for_each(|file| {
                // Load song
                let path = file.path();
                let mut song = Song::new(path).unwrap();
                let format = ffmpeg::format::input(&path).unwrap();

                // Get current metadata
                let mut title = String::default();
                let mut artist = String::default();
                let mut album = String::default();
                if let Some(t) = format.stream(0).unwrap().metadata().get("title") {
                    title = t.to_owned();
                }
                if let Some(a) = format.stream(0).unwrap().metadata().get("artist") {
                    artist = a.to_owned();
                }
                if let Some(a) = format.stream(0).unwrap().metadata().get("album") {
                    album = a.to_owned();
                }

                // perceptually hash song
                let mut phash = gen_phash(&song.analysis);
                if let Some(v) = db.get(phash.to_be_bytes()).unwrap() {
                    let stored: Stored = bincode::deserialize(&v).unwrap();

                    println!("HASH COLLISION!!! Assuming that its an dupe! Skipping...");
                    println!("--- Prev Song ---");
                    println!("{} - {}\t| {}", stored.artist, stored.title, stored.album);
                    println!("--- Curr Song ---");
                    println!("{} - {}\t| {}", artist, title, album);
                    println!("-----------------\n")
                } else {
                    // Start ingesting
                    println!("\n--- Curr Song ---");
                    println!(
                        "{} - {}\t| {}: {:x}_{:x}",
                        artist,
                        title,
                        album,
                        phash >> 32,
                        (phash << 96) >> 96
                    );
                    // get closest song
                    let (closest, closest_dist, tclosest, tclosest_dist) =
                        find_closest_song(&db, &title, &song.analysis);
                    if closest != 0 {
                        let closest_stored: Stored =
                            bincode::deserialize(&db.get(closest.to_be_bytes()).unwrap().unwrap())
                                .unwrap();
                        println!("--- Closest Song (Dist: {}) ---", closest_dist);
                        println!(
                            "{} - {}\t| {}: {:x}_{:x}",
                            closest_stored.artist,
                            closest_stored.title,
                            closest_stored.album,
                            closest >> 32,
                            (closest << 96) >> 96
                        );
                    }
                    if tclosest != 0 {
                        let tclosest_stored: Stored =
                            bincode::deserialize(&db.get(tclosest.to_be_bytes()).unwrap().unwrap())
                                .unwrap();
                        println!("--- Closest Title (Dist: {}) ---", tclosest_dist);
                        println!(
                            "{} - {}\t| {}: {:x}_{:x}",
                            tclosest_stored.artist,
                            tclosest_stored.title,
                            tclosest_stored.album,
                            tclosest >> 32,
                            (tclosest << 96) >> 96
                        );
                    }
                    println!("-----------------");

                    let process = requestty::prompt_one(
                        requestty::Question::expand("process")
                            .message("Choose process")
                            .choices(vec![
                                ('n', "New"),
                                ('r', "Replace closest"),
                                ('t', "Replace closest title"),
                                ('s', "Skip"),
                                ('x', "Abort"),
                            ])
                            .default('n')
                            .build(),
                    )
                    .unwrap();

                    if process.as_expand_item().unwrap().key == 'n' {
                        // Get new metadata
                        title = requestty::prompt_one(
                            requestty::Question::input("title")
                                .message("Song title")
                                .default(&title)
                                .build(),
                        )
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_owned();
                        artist = requestty::prompt_one(
                            requestty::Question::input("artist")
                                .message("Song artist")
                                .default(&artist)
                                .build(),
                        )
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_owned();
                        album = requestty::prompt_one(
                            requestty::Question::input("album")
                                .message("Song album")
                                .default(&album)
                                .build(),
                        )
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_owned();

                        let mut new_path =
                            msp.join(format!("{:x}_{:x}.opus", phash >> 32, (phash << 96) >> 96));

                        if sub_m.is_present("copy") {
                            // Copy over file
                            Command::new("ffmpeg")
                                .arg("-i")
                                .arg(&path.to_str().unwrap())
                                .arg("-map_metadata")
                                .arg("-1")
                                .arg("-metadata")
                                .arg(format!("TITLE={}", title))
                                .arg("-metadata")
                                .arg(format!("ARTIST={}", artist))
                                .arg("-metadata")
                                .arg(format!("ALBUM={}", album))
                                .arg("-f")
                                .arg("opus")
                                .arg("-c:a")
                                .arg("copy")
                                .arg("-vn")
                                .arg("-hide_banner")
                                .arg("-loglevel")
                                .arg("error")
                                .arg(&new_path.to_str().unwrap())
                                .spawn()
                                .unwrap()
                                .wait()
                                .unwrap();
                        } else {
                            let tmp_path = msp.join(format!("{:x}_{:x}.tmp", phash >> 32, (phash << 96) >> 96));
                            // Transcode over file
                            Command::new("ffmpeg")
                                .arg("-i")
                                .arg(&path.to_str().unwrap())
                                .arg("-map_metadata")
                                .arg("-1")
                                .arg("-metadata")
                                .arg(format!("TITLE={}", title))
                                .arg("-metadata")
                                .arg(format!("ARTIST={}", artist))
                                .arg("-metadata")
                                .arg(format!("ALBUM={}", album))
                                .arg("-f")
                                .arg("opus")
                                .arg("-c:a")
                                .arg("libopus")
                                .arg("-b:a")
                                .arg("160k")
                                .arg("-ar")
                                .arg("48k")
                                .arg("-vn")
                                .arg("-hide_banner")
                                .arg("-loglevel")
                                .arg("error")
                                .arg(&tmp_path.to_str().unwrap())
                                .spawn()
                                .unwrap()
                                .wait()
                                .unwrap();
                            // Recalculate perceptual hash
                            song = Song::new(&tmp_path.to_str().unwrap()).unwrap();
                            phash = gen_phash(&song.analysis);
                            new_path = msp.join(format!("{:x}_{:x}.opus", phash >> 32, (phash << 96) >> 96));
                            println!("New phash: {:x}_{:x}", phash >> 32, (phash << 96) >> 96);
                            // Move tmp file over to correct position
                            fs::rename(&tmp_path, &new_path).unwrap();
                        }

                        // r128gain song
                        Command::new("r128gain")
                            .arg("-v")
                            .arg("warning")
                            .arg(&new_path.to_str().unwrap())
                            .spawn()
                            .unwrap()
                            .wait()
                            .unwrap();

                        // Hash file
                        let mut file = fs::File::open(&new_path).unwrap();
                        let mut hasher = blake3::Hasher::new();
                        io::copy(&mut file, &mut hasher).unwrap();
                        let fhash = hasher.finalize().as_bytes().to_owned();

                        // Insert into db
                        db.insert(
                            phash.to_be_bytes(),
                            bincode::serialize(&Stored {
                                fhash,
                                phash,

                                title,
                                artist,
                                album,

                                analysis: song.analysis,
                            })
                            .unwrap(),
                        )
                        .unwrap();
                    } else if process.as_expand_item().unwrap().key == 'r' {
                    } else if process.as_expand_item().unwrap().key == 't' {
                    } else if process.as_expand_item().unwrap().key == 's' {
                    } else if process.as_expand_item().unwrap().key == 'x' {
                        db.flush().unwrap();
                        std::process::exit(0);
                    } else {
                        panic!("process not defined!");
                    }
                }
            });
    } else if let Some(sub_m) = matches.subcommand_matches("list") {
        db.iter().filter_map(|f| f.ok()).for_each(|(_, v)| {
            let stored: Stored = bincode::deserialize(&v).unwrap();
            if sub_m.is_present("detailed") {
                println!("{}", serde_json::to_string(&stored).unwrap());
            } else {
                println!("{:x}_{:x} | {} - {}\t| {}", stored.phash >> 32, (stored.phash << 96) >> 96, stored.artist, stored.title, stored.album);
            }
        });
    } else if let Some(_) = matches.subcommand_matches("sync") {
        db.iter().filter_map(|f| f.ok()).for_each(|(_, v)| {
            let stored: Stored = bincode::deserialize(&v).unwrap();

            let path = msp.join(format!(
                "{:x}_{:x}.opus",
                stored.phash >> 32,
                (stored.phash << 96) >> 96
            ));
            if path.exists() {
                // Load song
                let mut file = fs::File::open(&path).unwrap();

                // Compare current file hash against stored
                let mut hasher = blake3::Hasher::new();
                io::copy(&mut file, &mut hasher).unwrap();
                let fhash = hasher.finalize().as_bytes().to_owned();
                if fhash != stored.fhash {
                    println!(
                        "{:x}_{:x} - {} \t| stored differs from file! Updating...",
                        stored.phash >> 32,
                        (stored.phash << 96) >> 96,
                        stored.title
                    );

                    // Load song into ffmpeg
                    let format = ffmpeg::format::input(&path).unwrap();
                    let mut title = String::default();
                    let mut artist = String::default();
                    let mut album = String::default();
                    if let Some(t) = format.stream(0).unwrap().metadata().get("title") {
                        title = t.to_owned();
                    }
                    if let Some(a) = format.stream(0).unwrap().metadata().get("artist") {
                        artist = a.to_owned();
                    }
                    if let Some(a) = format.stream(0).unwrap().metadata().get("album") {
                        album = a.to_owned();
                    }

                    // Insert into db
                    db.insert(
                        stored.phash.to_be_bytes(),
                        bincode::serialize(&Stored {
                            fhash,
                            phash: stored.phash,

                            title,
                            artist,
                            album,

                            analysis: stored.analysis,
                        })
                        .unwrap(),
                    )
                    .unwrap();
                }
            } else {
                println!(
                    "{:x}_{:x} - {} \t| does exist anymore! Removing...",
                    stored.phash >> 32,
                    (stored.phash << 96) >> 96,
                    stored.title
                );
                db.remove(stored.phash.to_be_bytes()).unwrap();
            }
        });
    } else {
        println!("{}", matches.usage());
        std::process::exit(1);
    }

    db.flush()?;

    Ok(())
}

// Finds the closest matching song from the database
fn find_closest_song(db: &Db, title: &str, analysis: &Analysis) -> (u128, f32, u128, f32) {
    let mut closest: u128 = 0;
    let mut closest_dist = f32::MAX;

    let mut tclosest: u128 = 0;
    let mut tclosest_dist = 0.0;

    db.iter().filter_map(|f| f.ok()).for_each(|(_, v)| {
        let stored: Stored = bincode::deserialize(&v).unwrap();

        // Perceptually closer
        let dist = stored.analysis.custom_distance(analysis, cosine_distance);
        if dist < closest_dist {
            closest_dist = dist;
            closest = stored.phash;
        }

        // Title closer
        let tdist = fuzzy_compare(title, &stored.title);
        if tdist > tclosest_dist {
            tclosest_dist = tdist;
            tclosest = stored.phash;
        }
    });

    (closest, closest_dist, tclosest, tclosest_dist)
}

// Generate a sorta perceptual hash from a song analysis
fn gen_phash(analysis: &Analysis) -> u128 {
    let w0h0 = f16::from_f32(
        (analysis[AnalysisIndex::MeanLoudness]
            * analysis[AnalysisIndex::MeanSpectralCentroid]
            * analysis[AnalysisIndex::MeanSpectralFlatness]
            * analysis[AnalysisIndex::MeanSpectralRolloff])
            / (analysis[AnalysisIndex::MeanLoudness]
                + analysis[AnalysisIndex::MeanSpectralCentroid]
                + analysis[AnalysisIndex::MeanSpectralFlatness]
                + analysis[AnalysisIndex::MeanSpectralRolloff]),
    )
    .to_bits() as u16;
    let w0h1 = f16::from_f32(
        (analysis[AnalysisIndex::StdDeviationLoudness]
            * analysis[AnalysisIndex::StdDeviationSpectralCentroid]
            * analysis[AnalysisIndex::StdDeviationSpectralFlatness]
            * analysis[AnalysisIndex::StdDeviationSpectralRolloff])
            / (analysis[AnalysisIndex::StdDeviationLoudness]
                + analysis[AnalysisIndex::StdDeviationSpectralCentroid]
                + analysis[AnalysisIndex::StdDeviationSpectralFlatness]
                + analysis[AnalysisIndex::StdDeviationSpectralRolloff]),
    )
    .to_bits() as u16;
    let w0 = ((w0h0 as u32) << 16) | (w0h1 as u32);
    let w1 = (analysis[AnalysisIndex::Chroma1]
        + analysis[AnalysisIndex::Chroma3]
        + analysis[AnalysisIndex::Chroma5]
        + analysis[AnalysisIndex::Chroma7]
        + analysis[AnalysisIndex::Chroma9])
        .to_bits();
    let w2 = (analysis[AnalysisIndex::Chroma2]
        + analysis[AnalysisIndex::Chroma4]
        + analysis[AnalysisIndex::Chroma6]
        + analysis[AnalysisIndex::Chroma8]
        + analysis[AnalysisIndex::Chroma10])
        .to_bits();
    let w3 = (analysis[AnalysisIndex::Tempo] + analysis[AnalysisIndex::Zcr]).to_bits();

    ((w3 as u128) << 96) | ((w2 as u128) << 64) | ((w1 as u128) << 32) | (w0 as u128)
}
