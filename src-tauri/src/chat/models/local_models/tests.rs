#[test]
fn a_live_guess_says_a_repeated_phrase_once() {
    assert_eq!(
        collapse_repeats("Remind me Remind me Remind me Remind me"),
        "Remind me"
    );
    assert_eq!(
        collapse_repeats("Remind me to send, to send the report"),
        "Remind me to send, the report"
    );
    assert_eq!(
        collapse_repeats("Book a table for four"),
        "Book a table for four"
    );
    assert_eq!(collapse_repeats("Hello. hello"), "Hello.");
    assert_eq!(collapse_repeats(""), "");
}

use super::*;

#[test]
fn prompts_follow_each_models_format() {
    let msgs = vec![
        json!({"role": "system", "content": "Be brief."}),
        json!({"role": "user", "content": [{"type": "text", "text": "Hi"}]}),
        json!({"role": "assistant", "content": "Hello!"}),
        json!({"role": "tool", "content": "42"}),
        json!({"role": "user", "content": "Thanks"}),
    ];
    let t = turns(&msgs);
    assert_eq!(t[1], ("user".into(), "Hi".into()));
    assert_eq!(t[3], ("user".into(), "Tool result:\n42".into()));
    let chatml = render(Template::ChatMl, &t);
    assert!(chatml
        .starts_with("<|im_start|>system\nBe brief.<|im_end|>\n<|im_start|>user\nHi<|im_end|>"));
    assert!(chatml.ends_with("<|im_start|>assistant\n"));
    let llama = render(Template::Llama3, &t);
    assert!(llama.starts_with(
        "<|begin_of_text|><|start_header_id|>system<|end_header_id|>\n\nBe brief.<|eot_id|>"
    ));
    assert!(llama.ends_with("<|start_header_id|>assistant<|end_header_id|>\n\n"));
}

#[test]
fn mel_filters_match_whispers() {
    let f = mel_filters(80);
    assert_eq!(f.len(), 80 * 201);
    // Each filter is a triangle: non-negative, with weight somewhere, and the first rises from 0 Hz.
    assert!(f.iter().all(|v| *v >= 0.0));
    assert!((0..80).all(|m| f[m * 201..(m + 1) * 201].iter().any(|v| *v > 0.0)));
    assert_eq!(f[0], 0.0);
    assert!(f[1] > 0.0);
}

#[test]
fn wav_files_become_16khz_mono() {
    // 0.1 s of 8 kHz stereo 16-bit: left at half scale, right silent.
    let frames = 800u32;
    let mut wav = Vec::new();
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + frames * 4).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&2u16.to_le_bytes());
    wav.extend_from_slice(&8000u32.to_le_bytes());
    wav.extend_from_slice(&(8000u32 * 4).to_le_bytes());
    wav.extend_from_slice(&4u16.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(frames * 4).to_le_bytes());
    for _ in 0..frames {
        wav.extend_from_slice(&16384i16.to_le_bytes());
        wav.extend_from_slice(&0i16.to_le_bytes());
    }
    let s = wav_samples(&wav).unwrap();
    assert_eq!(s.len(), 1600);
    assert!((s[100] - 0.25).abs() < 1e-3);
    assert!(wav_samples(b"not audio").is_err());
}

/// With a model downloaded (`CORTEX_TEST_MODELS=<local data dir>`): a real answer and its speed.
#[test]
#[ignore]
fn a_downloaded_model_answers() {
    let Ok(dir) = std::env::var("CORTEX_TEST_MODELS") else {
        return;
    };
    let root = Path::new(&dir).join("models");
    let long_system = "You are a helpful assistant. ".repeat(
        std::env::var("CORTEX_TEST_SYSTEM_REPEAT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(1),
    );
    let msgs = vec![
        json!({"role": "system", "content": long_system}),
        json!({"role": "user", "content": "What is the capital of Portugal? One sentence."}),
    ];
    // Loaded once first, so the timing is of answering.
    generate(
        &root,
        "qwen2.5-0.5b",
        &msgs[1..],
        &Params {
            temperature: Some(0.0),
            top_p: None,
            max_tokens: Some(1),
        },
        |_| true,
    )
    .unwrap();
    let started = std::time::Instant::now();
    let mut first: Option<std::time::Duration> = None;
    let mut text = String::new();
    let usage = generate(
        &root,
        "qwen2.5-0.5b",
        &msgs,
        &Params {
            temperature: Some(0.0),
            top_p: None,
            max_tokens: Some(40),
        },
        |p| {
            first.get_or_insert(started.elapsed());
            text.push_str(p);
            true
        },
    )
    .unwrap();
    eprintln!(
        "prompt {} tokens, answer {} tokens, first piece after {:?}, all in {:?}: {text}",
        usage.prompt,
        usage.completion,
        first,
        started.elapsed()
    );
    assert!(text.contains("Lisbon"), "{text}");
}

/// Downloads the models the live tests use into `CORTEX_FETCH_MODELS` (as `CORTEX_TEST_MODELS`
/// expects them), through the app's own downloader; `CORTEX_FETCH_MODEL_IDS` narrows the list.
/// Models already there are kept.
#[tokio::test]
#[ignore]
async fn fetch_models_for_live_tests() {
    let Ok(dir) = std::env::var("CORTEX_FETCH_MODELS") else {
        return;
    };
    let root = Path::new(&dir).join("models");
    let ids = std::env::var("CORTEX_FETCH_MODEL_IDS")
        .unwrap_or_else(|_| "qwen2.5-0.5b,minilm-l6,whisper-tiny".into());
    for id in ids.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        if installed(&root, id) {
            eprintln!("{id}: already downloaded");
            continue;
        }
        let mut last = 0;
        download(&root, id, |done, total| {
            if done >= last + 50_000_000 || done == total {
                eprintln!("{id}: {} of {} MB", done / 1_000_000, total / 1_000_000);
                last = done;
            }
        })
        .await
        .unwrap();
        assert!(installed(&root, id), "{id} is complete");
    }
}

/// With Whisper downloaded (`CORTEX_TEST_MODELS`) and a recording (`CORTEX_TEST_WAV`).
#[test]
#[ignore]
fn a_downloaded_whisper_transcribes() {
    let (Ok(dir), Ok(wav)) = (
        std::env::var("CORTEX_TEST_MODELS"),
        std::env::var("CORTEX_TEST_WAV"),
    ) else {
        return;
    };
    let root = Path::new(&dir).join("models");
    let samples = audio_samples(&std::fs::read(wav).unwrap()).unwrap();
    let started = std::time::Instant::now();
    let text = transcribe(&root, "whisper-tiny", &samples, "", false).unwrap();
    eprintln!(
        "{:.1} s of audio in {:?}: {text}",
        samples.len() as f64 / 16_000.0,
        started.elapsed()
    );
    assert!(text.to_lowercase().contains("country"), "{text}");
}

#[test]
fn the_queue_has_a_limit_and_frees_places() {
    let held: Vec<Place> = (0..QUEUE_LIMIT).map_while(|_| Place::take()).collect();
    assert!(held.len() <= QUEUE_LIMIT);
    if held.len() == QUEUE_LIMIT {
        assert!(Place::take().is_none(), "full");
    }
    drop(held);
    assert!(Place::take().is_some());
}

/// With MiniLM downloaded (`CORTEX_TEST_MODELS`): similar sentences land close together.
#[test]
#[ignore]
fn a_downloaded_embedder_groups_meanings() {
    let Ok(dir) = std::env::var("CORTEX_TEST_MODELS") else {
        return;
    };
    let root = Path::new(&dir).join("models");
    let v = embed_texts(
        &root,
        "minilm-l6",
        &[
            "How do I cancel the contract?".into(),
            "Termination clause: either party may end the agreement with 30 days notice.".into(),
            "The bakery sells sourdough on Saturdays.".into(),
        ],
    )
    .unwrap();
    assert_eq!(v.len(), 3);
    assert_eq!(v[0].len(), 384);
    let dot = |a: &[f32], b: &[f32]| a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>();
    eprintln!(
        "contract~termination {:.3}, contract~bakery {:.3}",
        dot(&v[0], &v[1]),
        dot(&v[0], &v[2])
    );
    assert!(dot(&v[0], &v[1]) > dot(&v[0], &v[2]) + 0.1);
}

/// With Qwen 0.5B downloaded (`CORTEX_TEST_MODELS`): requests waiting together are answered
/// together, correctly, with more tokens per second and a first word sooner for the requests
/// that would otherwise wait their turn.
#[test]
#[ignore]
fn waiting_requests_are_batched() {
    let Ok(dir) = std::env::var("CORTEX_TEST_MODELS") else {
        return;
    };
    let root = Path::new(&dir).join("models");
    let questions = [
        "Name the three primary colours, then describe each in one sentence.",
        "What is the capital of France? Then give two facts about it.",
        "Count from one to ten in words, separated by commas.",
        "Say hello in Portuguese, then explain when to use it.",
    ];
    // (answer, tokens, time to the first piece)
    let ask = |q: &str, since: std::time::Instant| {
        let msgs = vec![json!({"role": "user", "content": q})];
        let mut text = String::new();
        let mut first = None;
        let usage = generate(
            &root,
            "qwen2.5-0.5b",
            &msgs,
            &Params {
                temperature: Some(0.0),
                top_p: None,
                max_tokens: Some(64),
            },
            |p| {
                first.get_or_insert_with(|| since.elapsed());
                text.push_str(p);
                true
            },
        )
        .unwrap();
        (text, usage.completion, first.unwrap_or_default())
    };
    ask("Warm up.", std::time::Instant::now());
    let started = std::time::Instant::now();
    let alone: Vec<_> = questions.iter().map(|q| ask(q, started)).collect();
    let serial = started.elapsed();
    let started = std::time::Instant::now();
    let together: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = questions
            .iter()
            .map(|q| s.spawn(move || ask(q, started)))
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let batched = started.elapsed();
    let rate = |rows: &[(String, usize, std::time::Duration)], took: std::time::Duration| {
        rows.iter().map(|r| r.1).sum::<usize>() as f64 / took.as_secs_f64()
    };
    let wait = |rows: &[(String, usize, std::time::Duration)]| {
        rows.iter().map(|r| r.2.as_secs_f64()).sum::<f64>() / rows.len() as f64
    };
    eprintln!(
        "one after another: {serial:?}, {:.1} tokens/s, first word after {:.2}s on average",
        rate(&alone, serial),
        wait(&alone)
    );
    eprintln!(
        "together:          {batched:?}, {:.1} tokens/s, first word after {:.2}s on average",
        rate(&together, batched),
        wait(&together)
    );
    for (a, b) in alone.iter().zip(&together) {
        eprintln!("alone:    {:?}\ntogether: {:?}", a.0, b.0);
    }
    let expect = [
        ["red", "blue"],
        ["Paris", "Paris"],
        ["three", "seven"],
        ["Olá", "Olá"],
    ];
    for ((text, _, _), words) in together.iter().zip(expect) {
        assert!(
            words.iter().all(|w| text.contains(w)),
            "a batched answer is right: {text}"
        );
    }
    assert!(
        wait(&together) < wait(&alone) * 0.8,
        "the first word comes sooner for requests that would wait"
    );
    assert!(
        rate(&together, batched) > rate(&alone, serial) * 0.85,
        "no slower overall"
    );
}

/// With Qwen 0.5B downloaded: padding is hidden from attention (which token pads makes no
/// difference) and a padded prompt picks the same next token as alone. The logits are not
/// bit-identical: the shifted positions change rounding, which quantization amplifies.
#[test]
#[ignore]
fn padding_is_neutral() {
    let Ok(dir) = std::env::var("CORTEX_TEST_MODELS") else {
        return;
    };
    let root = Path::new(&dir).join("models");
    let mut m = load(&root, "qwen2.5-0.5b").unwrap();
    let a = m
        .tokenizer
        .encode(
            "<|im_start|>user\nWhat is the capital of France?<|im_end|>\n<|im_start|>assistant\n",
            false,
        )
        .unwrap()
        .get_ids()
        .to_vec();
    let run = |m: &mut Loaded, ids: Vec<u32>, rows: usize, pads: &[usize]| {
        m.weights.clear();
        let width = ids.len() / rows;
        m.weights
            .forward(
                &Tensor::from_vec(ids, (rows, width), &Device::Cpu).unwrap(),
                0,
                pads,
            )
            .unwrap()
    };
    let alone = run(&mut m, a.clone(), 1, &[0]).get(0).unwrap();
    let best = |t: &Tensor| t.argmax(0).unwrap().to_scalar::<u32>().unwrap();
    for k in [1usize, 7] {
        let padded = |token: u32| {
            let mut ids = vec![token; k];
            ids.extend(&a);
            ids
        };
        let stop = m.stop[0];
        let one = run(&mut m, padded(stop), 1, &[k]).get(0).unwrap();
        let other = run(&mut m, padded(42), 1, &[k]).get(0).unwrap();
        let leak = (&one - &other)
            .unwrap()
            .abs()
            .unwrap()
            .max(0)
            .unwrap()
            .to_scalar::<f32>()
            .unwrap();
        assert_eq!(leak, 0.0, "padding is invisible");
        assert_eq!(best(&one), best(&alone), "same next token with {k} pads");
    }
}

#[test]
fn the_catalog_is_well_formed() {
    for e in CATALOG {
        assert!(e.files.iter().any(|f| f.2 == "tokenizer.json"), "{}", e.id);
        match e.kind {
            Kind::Chat => assert!(e.files.iter().any(|f| f.2 == "model.gguf")),
            Kind::Speech | Kind::Embedding => assert!(
                e.files.iter().any(|f| f.2 == "model.safetensors")
                    && e.files.iter().any(|f| f.2 == "config.json")
            ),
        }
    }
    let dir = tempfile::tempdir().unwrap();
    assert!(list(dir.path()).iter().all(|v| !v.installed));
    assert!(models(dir.path()).is_empty());
}
