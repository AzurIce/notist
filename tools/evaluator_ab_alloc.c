/* Linux/glibc-only, whole-Rust-process allocation API observer.
 * No dlsym recursion and no allocator policy changes. Diagnostic only.
 * Aligned allocation entrypoints and libc-internal calls are not counted.
 */
#define _GNU_SOURCE
#include <stdatomic.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

extern void *__libc_malloc(size_t);
extern void *__libc_calloc(size_t, size_t);
extern void *__libc_realloc(void *, size_t);
extern void __libc_free(void *);
static _Atomic int enabled;
static _Atomic unsigned long long allocations, requested, releases, reallocations;

static void observe(void *p, size_t size, int reallocation) {
    if (p && atomic_load_explicit(&enabled, memory_order_relaxed)) {
        atomic_fetch_add_explicit(&allocations, 1, memory_order_relaxed);
        atomic_fetch_add_explicit(&requested, size, memory_order_relaxed);
        if (reallocation) atomic_fetch_add_explicit(&reallocations, 1, memory_order_relaxed);
    }
}
void *malloc(size_t n) { void *p = __libc_malloc(n); observe(p, n, 0); return p; }
void *calloc(size_t a, size_t b) { void *p = __libc_calloc(a, b); observe(p, a * b, 0); return p; }
void *realloc(void *old, size_t n) { void *p = __libc_realloc(old, n); observe(p, n, 1); return p; }
void free(void *p) {
    if (p && atomic_load_explicit(&enabled, memory_order_relaxed)) atomic_fetch_add_explicit(&releases, 1, memory_order_relaxed);
    __libc_free(p);
}
__attribute__((constructor)) static void start(void) {
    char executable[4096];
    const char *target = getenv("EVAL_AB_ALLOC_TARGET");
    ssize_t size = readlink("/proc/self/exe", executable, sizeof(executable) - 1);
    if (size < 0 || !target) return;
    executable[size] = 0;
    if (strcmp(executable, target) == 0) atomic_store(&enabled, 1);
}
__attribute__((destructor)) static void finish(void) {
    if (!atomic_exchange(&enabled, 0)) return;
    char buffer[512];
    int size = snprintf(buffer, sizeof(buffer), "[eval-native-alloc] pid=%ld allocations=%llu requested_bytes=%llu frees=%llu reallocations=%llu\n", (long)getpid(), atomic_load(&allocations), atomic_load(&requested), atomic_load(&releases), atomic_load(&reallocations));
    if (size > 0 && (size_t)size < sizeof(buffer)) write(STDERR_FILENO, buffer, size);
}
