//! Scoped operation counts, not a build-time benchmark. Only allocations on
//! the measured test thread are counted, so tests can run concurrently.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    hint::black_box,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Allocations {
    allocations: usize,
    reallocations: usize,
}

thread_local! {
    static COUNTS: Cell<Option<Allocations>> = const { Cell::new(None) };
}

struct CountingAllocator;

fn count(reallocation: bool) {
    let _ = COUNTS.try_with(|counts| {
        if let Some(mut current) = counts.get() {
            if reallocation {
                current.reallocations += 1;
            } else {
                current.allocations += 1;
            }
            counts.set(Some(current));
        }
    });
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            count(false);
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            count(false);
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        if !pointer.is_null() {
            count(true);
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn measure<T>(run: impl FnOnce() -> T) -> (T, Allocations) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            COUNTS.set(None);
        }
    }
    assert!(COUNTS.replace(Some(Allocations::default())).is_none());
    let reset = Reset;
    let result = run();
    let counts = COUNTS.get().unwrap();
    drop(reset);
    (result, counts)
}

#[test]
fn string_token_scanning_does_not_allocate() {
    for source in [
        format!("\"{}\" trailing", "中文🙂a".repeat(1024)),
        format!("r###\"{}\"### trailing", "中文\\\"##🙂".repeat(1024)),
        format!("\"\"\"{}\"\"\" trailing", "中文\r\n\\🙂\"\"".repeat(1024)),
        format!("r##\"\"\"{}", "中文\n🙂\"\"\"#".repeat(1024)),
    ] {
        let (_, counts) =
            measure(|| notist_syntax::code::lexer::literal_token(black_box(&source), 0));
        assert_eq!(counts, Allocations::default());
    }
}

#[test]
fn url_safety_checks_do_not_allocate_even_with_controls() {
    for (url, safe) in [
        ("https://example.com/中文?query#fragment", true),
        ("\0\u{1f} \th\tt\nt\rpS://example.com/中文", true),
        ("relative/path/中文", true),
        ("\tja\nva\rscript:alert(1)", false),
        ("d\ta\nt\ra:text/html,unsafe", false),
    ] {
        let (result, counts) = measure(|| notist_html::is_safe_url(black_box(url)));
        assert_eq!(result, safe);
        assert_eq!(counts, Allocations::default());
    }
}

#[test]
fn borrowed_attributes_and_word_arrays_add_no_temporary_allocations() {
    use notist_core::item::{Ctor, Item, Value};

    let plain =
        Item::new(Ctor::Math, Default::default()).with_field("text", Value::Str("x".into()));
    let mut attributed = plain.clone();
    for key in ["id", "title", "lang", "dir", "aria-label", "data-label"] {
        attributed.attrs.insert(key, Value::Str("中文<&\"".into()));
    }
    for key in ["class", "tags"] {
        attributed.attrs.insert(
            key,
            Value::Array((0..128).map(|_| Value::Str("word".into())).collect()),
        );
    }
    let renderer = notist_html::Renderer::new();
    // Initialize any renderer metadata before entering the measured scopes.
    black_box(renderer.render(&plain));
    black_box(renderer.render(&attributed));
    let (_, plain_counts) = measure(|| renderer.render(black_box(&plain)));
    let (html, attributed_counts) = measure(|| renderer.render(black_box(&attributed)));
    assert!(html.contains("class=\"notist-math word word"));
    assert!(html.contains("data-notist-tags=\"word word"));
    // Output growth may reallocate; attributes must not create other buffers.
    assert_eq!(attributed_counts.allocations, plain_counts.allocations);
}
