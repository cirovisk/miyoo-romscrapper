use rom_art_rs::{
    engine::{Settings, generate_local_gamelist, run_with_provider},
    provider::{
        ArtworkType, Candidate, Credentials, Media, Provider, ServiceStopped, choose_media,
        exact_match,
    },
    worker::{Event, ResolveOptions, Resolved, ReviewAction, resolve},
};
use std::{
    fs,
    io::{Cursor, Read, Write},
    net::TcpListener,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

fn server(
    build: impl FnOnce(&str) -> Vec<(u16, Vec<u8>)>,
) -> (String, thread::JoinHandle<Vec<String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let responses = build(&base);
    let handle = thread::spawn(move || {
        let mut requests = Vec::new();
        for (status, body) in responses {
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(e)
                        if e.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10))
                    }
                    Err(e) => panic!("Mock server: {e}"),
                }
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            requests.push(String::from_utf8(request).unwrap());
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        }
        requests
    });
    (base, handle)
}
fn json(data: serde_json::Value) -> (u16, Vec<u8>) {
    (
        200,
        serde_json::to_vec(&serde_json::json!({"success":true,"data":data})).unwrap(),
    )
}
fn credentials() -> Credentials {
    Credentials {
        api_key: "TEST_SECRET".into(),
    }
}

#[test]
fn matches_require_unique_exact_title() {
    let a = Candidate {
        id: 1,
        name: "Game".into(),
    };
    let b = Candidate {
        id: 2,
        name: "Game Remastered".into(),
    };
    assert_eq!(
        exact_match(" game ", &[a.clone(), b.clone()]).unwrap().id,
        1
    );
    assert!(exact_match("Game", &[b]).is_none());
    assert!(exact_match("Game", &[a.clone(), Candidate { id: 3, ..a }]).is_none());
}

#[test]
fn automatic_selection_skips_results_without_an_exact_title_match() {
    let (base, handle) =
        server(|_| vec![json(serde_json::json!([{"id":7,"name":"Different Game"}]))]);
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let review_shown = std::sync::atomic::AtomicBool::new(false);
    let selected = resolve(
        &provider,
        Path::new("Unknown.nes"),
        ResolveOptions {
            auto_first_result: true,
            choose_artwork: false,
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            arcade_root: None,
            arcade_mode: Arc::new(AtomicBool::new(false)),
        },
        &AtomicBool::new(false),
        &|event| {
            if matches!(event, Event::Review { .. }) {
                review_shown.store(true, Ordering::Relaxed);
            }
        },
    )
    .unwrap();

    assert!(matches!(selected, Some(Resolved::NoMatch)));
    assert!(!review_shown.load(Ordering::Relaxed));
    assert_eq!(handle.join().unwrap().len(), 1);
}

#[test]
fn manual_skip_without_a_title_match_is_counted_as_unmatched() {
    let (base, handle) = server(|_| vec![json(serde_json::json!([]))]);
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let selected = resolve(
        &provider,
        Path::new("Unknown.nes"),
        ResolveOptions {
            auto_first_result: false,
            choose_artwork: false,
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            arcade_root: None,
            arcade_mode: Arc::new(AtomicBool::new(false)),
        },
        &AtomicBool::new(false),
        &|event| {
            if let Event::Review { answer, .. } = event {
                answer.send(ReviewAction::Skip).unwrap();
            }
        },
    )
    .unwrap();

    assert!(matches!(selected, Some(Resolved::NoMatch)));
    assert_eq!(handle.join().unwrap().len(), 1);
}

#[test]
fn batch_summary_counts_roms_without_a_game_match() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("Unknown.nes"), b"rom").unwrap();
    let (base, handle) =
        server(|_| vec![json(serde_json::json!([{"id":7,"name":"Different Game"}]))]);
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let no_match = std::sync::atomic::AtomicUsize::new(0);
    let settings = Settings {
        root: root.path().to_path_buf(),
        output: root.path().join("Artwork"),
        credentials: credentials(),
        auto_first_result: true,
        gamelist: false,
        ..Default::default()
    };

    run_with_provider(settings, &provider, &AtomicBool::new(false), |event| {
        if let Event::Summary {
            no_match: count, ..
        } = event
        {
            no_match.store(count, Ordering::Relaxed);
        }
    })
    .unwrap();

    assert_eq!(no_match.load(Ordering::Relaxed), 1);
    assert_eq!(handle.join().unwrap().len(), 1);
}

#[test]
fn chooses_orientation_then_score_and_handles_empty() {
    let media = serde_json::from_value(serde_json::json!([
        {"url":"wide","width":920,"height":430,"score":100},
        {"url":"portrait","width":600,"height":900,"score":3},
        {"url":"best","width":600,"height":900,"score":7}
    ]))
    .unwrap();
    assert_eq!(choose_media(media, 250, 360).unwrap().url, "best");
    assert!(choose_media(vec![], 250, 360).is_err());
}

#[test]
fn full_download_removes_temporary_cache_and_preserves_outputs() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("Imgs");
    fs::create_dir(&output).unwrap();
    fs::write(
        root.path().join("A & B (USA).zip"),
        b"not even a zip: no payload inspection needed",
    )
    .unwrap();
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(20, 10)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let image_bytes = png.into_inner();
    let image_bytes_again = image_bytes.clone();
    let (base, handle) = server(|base| {
        let search = json(serde_json::json!([{"id":7,"name":"A & B"}]));
        let media = json(
            serde_json::json!([{"url":format!("{base}cdn/image.png"),"width":20,"height":10,"score":5}]),
        );
        vec![
            search.clone(),
            media.clone(),
            (200, image_bytes),
            search,
            media,
            (200, image_bytes_again),
        ]
    });
    let mut settings = Settings {
        root: root.path().into(),
        output,
        credentials: credentials(),
        ..Default::default()
    };
    let provider = Provider::with_base(credentials(), &base).unwrap();
    run_with_provider(settings.clone(), &provider, &AtomicBool::new(false), |_| {}).unwrap();
    assert!(!settings.output.join(".rom-art-cache").exists());
    let path = settings.output.join("A & B (USA).zip.png");
    assert_eq!(image::image_dimensions(&path).unwrap(), (250, 360));
    let first = fs::read(&path).unwrap();
    let xml = fs::read(root.path().join("gamelist.xml")).unwrap();
    let doc = xmltree::Element::parse(xml.as_slice()).unwrap();
    assert_eq!(
        doc.get_child("game")
            .unwrap()
            .get_child("image")
            .unwrap()
            .get_text()
            .unwrap(),
        "./Imgs/A & B (USA).zip.png"
    );
    // The temporary image cache is removed after each operation, so the overwritten image is fetched again.
    settings.overwrite = true;
    run_with_provider(settings.clone(), &provider, &AtomicBool::new(false), |_| {}).unwrap();
    assert!(!settings.output.join(".rom-art-cache").exists());
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 6);
    assert!(requests[0].contains("/search/autocomplete/A%20&%20B"));
    assert!(requests[1].contains("/grids/game/7?"));
    assert!(requests[1].contains("types=static"));
    for index in [0, 1, 3, 4] {
        assert!(
            requests[index]
                .to_lowercase()
                .contains("authorization: bearer test_secret")
        );
    }
    assert!(!requests[2].to_lowercase().contains("authorization"));
    assert!(!requests[5].to_lowercase().contains("authorization"));
    // Server stopped: preserved outputs must not cause network calls.
    settings.overwrite = false;
    run_with_provider(settings, &provider, &AtomicBool::new(false), |_| {}).unwrap();
    assert_eq!(fs::read(path).unwrap(), first);
    assert_eq!(fs::read(root.path().join("gamelist.xml")).unwrap(), xml);
}

#[test]
fn artwork_download_can_skip_automatic_gamelist_export() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("Artwork");
    fs::write(root.path().join("Game.gba"), b"rom").unwrap();
    let mut png = Cursor::new(Vec::new());
    image::DynamicImage::new_rgb8(20, 10)
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
    let (base, handle) = server(|base| {
        vec![
            json(serde_json::json!([{"id":7,"name":"Game"}])),
            json(
                serde_json::json!([{"url":format!("{base}cdn/image.png"),"width":20,"height":10,"score":5}]),
            ),
            (200, png.into_inner()),
        ]
    });
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let settings = Settings {
        root: root.path().into(),
        output: output.clone(),
        credentials: credentials(),
        gamelist: false,
        ..Default::default()
    };

    run_with_provider(settings, &provider, &AtomicBool::new(false), |_| {}).unwrap();

    assert!(output.join("Game.gba.png").is_file());
    assert!(!root.path().join("gamelist.xml").exists());
    assert!(!root.path().join("miyoogamelist.xml").exists());
    assert_eq!(handle.join().unwrap().len(), 3);
}

#[test]
fn artwork_gallery_loads_thumbnail_urls_and_keeps_full_image_selection() {
    let image_bytes = {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(20, 10)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        png.into_inner()
    };
    let image_bytes_again = image_bytes.clone();
    let (base, handle) = server(|base| {
        vec![
            json(serde_json::json!([{"id":7,"name":"Game"}])),
            json(serde_json::json!([
                {"url":format!("{base}cdn/full-1.png"),"thumb":format!("{base}cdn/thumb-1.png"),"width":20,"height":10,"score":10},
                {"url":format!("{base}cdn/full-2.png"),"thumb":format!("{base}cdn/thumb-2.png"),"width":20,"height":10,"score":5}
            ])),
            (200, image_bytes),
            (200, image_bytes_again),
        ]
    });
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let selected = resolve(
        &provider,
        Path::new("Game.nes"),
        ResolveOptions {
            auto_first_result: true,
            choose_artwork: true,
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            arcade_root: None,
            arcade_mode: Arc::new(AtomicBool::new(false)),
        },
        &AtomicBool::new(false),
        &|event| {
            if let Event::ArtworkReview { media, answer, .. } = event {
                answer
                    .send(ReviewAction::SelectArtwork(media[1].clone()))
                    .unwrap();
            }
        },
    )
    .unwrap();

    assert!(matches!(
        selected,
        Some(Resolved::Game(_, Some(Media { url, .. }))) if url == format!("{base}cdn/full-2.png")
    ));
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(
        requests
            .iter()
            .any(|request| request.contains("/cdn/thumb-1.png"))
    );
    assert!(
        requests
            .iter()
            .any(|request| request.contains("/cdn/thumb-2.png"))
    );
    assert!(
        !requests
            .iter()
            .any(|request| request.contains("/cdn/full-"))
    );
}

#[test]
fn ambiguous_result_can_be_selected_manually() {
    let (base, handle) = server(|_| {
        vec![json(
            serde_json::json!([{"id":1,"name":"Game"},{"id":2,"name":"Game"}]),
        )]
    });
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let selected = resolve(
        &provider,
        Path::new("Game.nes"),
        ResolveOptions {
            auto_first_result: false,
            choose_artwork: false,
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            arcade_root: None,
            arcade_mode: Arc::new(AtomicBool::new(false)),
        },
        &AtomicBool::new(false),
        &|event| {
            if let Event::Review {
                candidates, answer, ..
            } = event
            {
                answer
                    .send(ReviewAction::Select(candidates[1].clone()))
                    .unwrap();
            }
        },
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        selected,
        Resolved::Game(Candidate { id: 2, .. }, None)
    ));
    handle.join().unwrap();
}

#[test]
fn arcade_mode_researches_catalog_title_and_saves_alias() {
    let root = tempfile::tempdir().unwrap();
    let rom = root.path().join("mslug.zip");
    fs::write(&rom, b"rom").unwrap();
    fs::write(
        root.path().join("fbneo.dat"),
        b"game ( name \"mslug\" description \"Metal Slug\" )",
    )
    .unwrap();
    let (base, handle) = server(|_| {
        vec![
            json(serde_json::json!([])),
            json(serde_json::json!([{"id":7,"name":"Metal Slug"}])),
        ]
    });
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let selected = resolve(
        &provider,
        &rom,
        ResolveOptions {
            auto_first_result: false,
            choose_artwork: false,
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            arcade_root: Some(root.path().to_path_buf()),
            arcade_mode: Arc::new(AtomicBool::new(false)),
        },
        &AtomicBool::new(false),
        &|event| {
            if let Event::Review {
                candidates, answer, ..
            } = event
            {
                if candidates.is_empty() {
                    let _ = answer.send(ReviewAction::SetArcade(true));
                } else {
                    answer
                        .send(ReviewAction::Select(candidates[0].clone()))
                        .unwrap();
                }
            }
        },
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        selected,
        Resolved::Game(Candidate { id: 7, .. }, None)
    ));
    let requests = handle.join().unwrap();
    assert!(requests[0].contains("/search/autocomplete/mslug"));
    assert!(requests[1].contains("/search/autocomplete/Metal%20Slug"));
    assert_eq!(
        rom_art_rs::arcade::title(root.path(), &rom).unwrap(),
        Some("Metal Slug".into())
    );
}

#[test]
fn initially_enabled_arcade_mode_searches_the_catalog_title() {
    let root = tempfile::tempdir().unwrap();
    let rom = root.path().join("mslug.zip");
    fs::write(&rom, b"rom").unwrap();
    fs::write(
        root.path().join("fbneo.dat"),
        b"game ( name \"mslug\" description \"Metal Slug\" )",
    )
    .unwrap();
    let (base, handle) = server(|_| vec![json(serde_json::json!([{"id":7,"name":"Metal Slug"}]))]);
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let selected = resolve(
        &provider,
        &rom,
        ResolveOptions {
            auto_first_result: true,
            choose_artwork: false,
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            arcade_root: Some(root.path().to_path_buf()),
            arcade_mode: Arc::new(AtomicBool::new(true)),
        },
        &AtomicBool::new(false),
        &|_| {},
    )
    .unwrap();

    assert!(matches!(
        selected,
        Some(Resolved::Game(Candidate { id: 7, .. }, None))
    ));
    assert!(handle.join().unwrap()[0].contains("/search/autocomplete/Metal%20Slug"));
}

#[test]
fn unmatched_game_supports_research_and_cancellation() {
    let (base, handle) = server(|_| {
        vec![
            json(serde_json::json!([])),
            json(serde_json::json!([{"id":2,"name":"New title"}])),
        ]
    });
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let selected = resolve(
        &provider,
        Path::new("Unknown.nes"),
        ResolveOptions {
            auto_first_result: false,
            choose_artwork: false,
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            arcade_root: None,
            arcade_mode: Arc::new(AtomicBool::new(false)),
        },
        &AtomicBool::new(false),
        &|event| {
            if let Event::Review {
                candidates, answer, ..
            } = event
            {
                if candidates.is_empty() {
                    answer
                        .send(ReviewAction::Search("New title".into()))
                        .unwrap();
                } else {
                    answer
                        .send(ReviewAction::Select(candidates[0].clone()))
                        .unwrap();
                }
            }
        },
    )
    .unwrap()
    .unwrap();
    assert!(matches!(
        selected,
        Resolved::Game(Candidate { id: 2, .. }, None)
    ));
    handle.join().unwrap();
    let (base, handle) = server(|_| vec![json(serde_json::json!([]))]);
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let stop = AtomicBool::new(false);
    let error = resolve(
        &provider,
        Path::new("Unknown.nes"),
        ResolveOptions {
            auto_first_result: false,
            choose_artwork: false,
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            arcade_root: None,
            arcade_mode: Arc::new(AtomicBool::new(false)),
        },
        &stop,
        &|_| stop.store(true, Ordering::Relaxed),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "Cancelled");
    handle.join().unwrap();
}

#[test]
fn artwork_gallery_can_return_to_game_selection_without_researching() {
    let (base, handle) = server(|base| {
        let media = || {
            json(serde_json::json!([{
                "url":format!("{base}cdn/full.png"),
                "thumb":format!("{base}cdn/thumb.png"),
                "width":250,
                "height":360,
                "score":1
            }]))
        };
        vec![
            json(serde_json::json!([
                {"id":1,"name":"Game"},
                {"id":2,"name":"Game Alternative"}
            ])),
            media(),
            (200, b"not an image".to_vec()),
            media(),
            (200, b"not an image".to_vec()),
        ]
    });
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let review_count = std::sync::atomic::AtomicUsize::new(0);
    let artwork_game = std::sync::atomic::AtomicUsize::new(0);
    let selected = resolve(
        &provider,
        Path::new("Game.nes"),
        ResolveOptions {
            auto_first_result: false,
            choose_artwork: true,
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            arcade_root: None,
            arcade_mode: Arc::new(AtomicBool::new(false)),
        },
        &AtomicBool::new(false),
        &|event| match event {
            Event::Review {
                candidates, answer, ..
            } => {
                let attempt = review_count.fetch_add(1, Ordering::Relaxed);
                let candidate = if attempt == 0 {
                    candidates[0].clone()
                } else {
                    candidates[1].clone()
                };
                answer.send(ReviewAction::Select(candidate)).unwrap();
            }
            Event::ArtworkReview { game, answer, .. } => {
                artwork_game.store(game.id as usize, Ordering::Relaxed);
                let action = if game.id == 1 {
                    ReviewAction::BackToGame
                } else {
                    ReviewAction::Skip
                };
                answer.send(action).unwrap();
            }
            _ => {}
        },
    )
    .unwrap();

    assert!(selected.is_none());
    assert_eq!(review_count.load(Ordering::Relaxed), 2);
    assert_eq!(artwork_game.load(Ordering::Relaxed), 2);
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 5);
    assert!(requests[0].contains("/search/autocomplete/Game"));
    assert!(requests[1].contains("/grids/game/1"));
    assert!(requests[3].contains("/grids/game/2"));
}

#[test]
fn artwork_unavailable_dialog_can_return_to_game_selection() {
    let (base, handle) = server(|_| {
        vec![
            json(serde_json::json!([
                {"id":1,"name":"Game"},
                {"id":2,"name":"Game Alternative"}
            ])),
            json(serde_json::json!([])),
            json(serde_json::json!([])),
        ]
    });
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let review_count = std::sync::atomic::AtomicUsize::new(0);
    let selected_game = std::sync::atomic::AtomicUsize::new(0);
    let result = resolve(
        &provider,
        Path::new("Game.nes"),
        ResolveOptions {
            auto_first_result: false,
            choose_artwork: true,
            kind: ArtworkType::Grid,
            width: 250,
            height: 360,
            arcade_root: None,
            arcade_mode: Arc::new(AtomicBool::new(false)),
        },
        &AtomicBool::new(false),
        &|event| {
            if let Event::Review {
                candidates,
                query,
                answer,
                ..
            } = event
            {
                let step = review_count.fetch_add(1, Ordering::Relaxed);
                if query.starts_with("Artwork unavailable:") {
                    let action = if step == 1 {
                        ReviewAction::BackToGame
                    } else {
                        ReviewAction::Skip
                    };
                    answer.send(action).unwrap();
                } else {
                    let candidate = candidates[if step == 0 { 0 } else { 1 }].clone();
                    selected_game.store(candidate.id as usize, Ordering::Relaxed);
                    answer.send(ReviewAction::Select(candidate)).unwrap();
                }
            }
        },
    )
    .unwrap();

    assert!(result.is_none());
    assert_eq!(selected_game.load(Ordering::Relaxed), 2);
    assert_eq!(review_count.load(Ordering::Relaxed), 4);
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 3);
    assert!(requests[0].contains("/search/autocomplete/Game"));
    assert!(requests[1].contains("/grids/game/1"));
    assert!(requests[2].contains("/grids/game/2"));
}

#[test]
fn failures_do_not_leak_keys_and_unsuccessful_json_is_rejected() {
    for status in [401, 403, 429, 503] {
        let (base, handle) = server(|_| vec![(status, b"TEST_SECRET".to_vec())]);
        let provider = Provider::with_base(credentials(), &base).unwrap();
        let error = provider
            .search("Game", &AtomicBool::new(false))
            .unwrap_err();
        assert!(error.downcast_ref::<ServiceStopped>().is_some());
        assert!(!format!("{error:?}").contains("TEST_SECRET"));
        handle.join().unwrap();
    }
    let (base, handle) = server(|_| {
        vec![(
            200,
            br#"{"success":false,"errors":["TEST_SECRET"]}"#.to_vec(),
        )]
    });
    let error = Provider::with_base(credentials(), &base)
        .unwrap()
        .search("Game", &AtomicBool::new(false))
        .unwrap_err();
    assert!(!format!("{error:?}").contains("TEST_SECRET"));
    handle.join().unwrap();
}

#[test]
fn local_gamelist_needs_no_api_key_and_filters_hidden_files() {
    let root = tempfile::tempdir().unwrap();
    let output = root.path().join("Imgs");
    fs::create_dir(&output).unwrap();
    fs::create_dir(root.path().join(".hidden")).unwrap();
    for name in ["Game.NES", "notes.txt", ".hidden/Other.nes"] {
        fs::write(root.path().join(name), b"rom").unwrap();
    }
    let settings = Settings {
        root: root.path().into(),
        output,
        ..Default::default()
    };
    generate_local_gamelist(settings, &AtomicBool::new(false), |_| {}).unwrap();
    let bytes = fs::read(root.path().join("gamelist.xml")).unwrap();
    let doc = xmltree::Element::parse(bytes.as_slice()).unwrap();
    assert_eq!(
        doc.children
            .iter()
            .filter(|n| matches!(n, xmltree::XMLNode::Element(_)))
            .count(),
        1
    );
}

#[test]
fn onion_local_export_uses_imgs_and_both_lists_without_a_key() {
    let root = tempfile::tempdir().unwrap();
    let rom = root.path().join("Game (USA).gba");
    fs::write(&rom, b"rom").unwrap();
    let mut settings = Settings {
        root: root.path().into(),
        gamelist_format: rom_art_rs::gamelist::Format::Both,
        ..Default::default()
    };
    settings.onion_preset(250, 360);
    fs::create_dir(settings.image_directory()).unwrap();
    image::DynamicImage::new_rgba8(250, 360)
        .save(settings.image_directory().join("Game (USA).png"))
        .unwrap();
    generate_local_gamelist(settings.clone(), &AtomicBool::new(false), |_| {}).unwrap();
    for name in ["gamelist.xml", "miyoogamelist.xml"] {
        let bytes = fs::read(root.path().join(name)).unwrap();
        let doc = xmltree::Element::parse(bytes.as_slice()).unwrap();
        assert_eq!(
            doc.get_child("game")
                .unwrap()
                .get_child("image")
                .unwrap()
                .get_text()
                .unwrap(),
            "./Imgs/Game (USA).png"
        );
    }
    fs::write(root.path().join("Game (USA).zip"), b"rom").unwrap();
    assert!(generate_local_gamelist(settings, &AtomicBool::new(false), |_| {}).is_err());
}
