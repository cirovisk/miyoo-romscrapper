use rom_art_rs::{
    engine::{Event, ReviewAction, Settings, generate_local_gamelist, resolve, run_with_provider},
    provider::{Candidate, Credentials, Provider, ServiceStopped, choose_media, exact_match},
};
use std::{
    fs,
    io::{Cursor, Read, Write},
    net::TcpListener,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
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
fn full_download_auth_cache_gamelist_and_preservation() {
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
    let (base, handle) = server(|base| {
        let search = json(serde_json::json!([{"id":7,"name":"A & B"}]));
        let media = json(
            serde_json::json!([{"url":format!("{base}cdn/image.png"),"width":20,"height":10,"score":5}]),
        );
        vec![
            search.clone(),
            media.clone(),
            (200, png.into_inner()),
            search,
            media,
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
    // Overwrite uses the media cache: API metadata still fetched, PNG is not fetched again.
    settings.overwrite = true;
    run_with_provider(settings.clone(), &provider, &AtomicBool::new(false), |_| {}).unwrap();
    let requests = handle.join().unwrap();
    assert_eq!(requests.len(), 5);
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
    // Server stopped: preserved outputs must not cause network calls.
    settings.overwrite = false;
    run_with_provider(settings, &provider, &AtomicBool::new(false), |_| {}).unwrap();
    assert_eq!(fs::read(path).unwrap(), first);
    assert_eq!(fs::read(root.path().join("gamelist.xml")).unwrap(), xml);
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
    assert_eq!(selected.id, 2);
    handle.join().unwrap();
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
        &AtomicBool::new(false),
        &|event| {
            if let Event::Review { answer, .. } = event {
                answer
                    .send(ReviewAction::Search("New title".into()))
                    .unwrap();
            }
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(selected.id, 2);
    handle.join().unwrap();
    let (base, handle) = server(|_| vec![json(serde_json::json!([]))]);
    let provider = Provider::with_base(credentials(), &base).unwrap();
    let stop = AtomicBool::new(false);
    let error = resolve(&provider, Path::new("Unknown.nes"), &stop, &|_| {
        stop.store(true, Ordering::Relaxed)
    })
    .unwrap_err();
    assert_eq!(error.to_string(), "Cancelled");
    handle.join().unwrap();
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
