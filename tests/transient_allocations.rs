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
