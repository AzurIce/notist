/* Linux/glibc-only diagnostic. Observe the existing allocator; never replace it.
 * Used only in disposable after-only runs, not ordinary performance samples. */
#define _GNU_SOURCE
#include <dirent.h>
#include <dlfcn.h>
#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <sys/syscall.h>
#include <time.h>
#include <unistd.h>

extern void *__libc_malloc(size_t);
static DIR *(*real_opendir)(const char *);
static __thread const char *directory_path;
static __thread int recording;
static __thread unsigned malloc_calls;

static uint64_t nanos(clockid_t clock) {
    struct timespec value;
    if (clock_gettime(clock, &value)) return 0;
    return (uint64_t)value.tv_sec * 1000000000ULL + value.tv_nsec;
}

static void report(const char *kind, size_t size, uint64_t wall, uint64_t cpu) {
    char buffer[1024];
    recording = 1;
    int length = snprintf(buffer, sizeof(buffer),
        "[residual-allocator] pid=%ld kind=%s size=%zu malloc_calls=%u wall_ns=%llu cpu_ns=%llu path=%s\n",
        (long)getpid(), kind, size, malloc_calls, (unsigned long long)wall,
        (unsigned long long)cpu, directory_path ? directory_path : "");
    if (length > 0) {
        size_t count = (size_t)length < sizeof(buffer) ? (size_t)length : sizeof(buffer) - 1;
        (void)syscall(SYS_write, STDERR_FILENO, buffer, count);
    }
    recording = 0;
}

void *malloc(size_t size) {
    if (!directory_path || recording) return __libc_malloc(size);
    uint64_t wall = nanos(CLOCK_MONOTONIC), cpu = nanos(CLOCK_THREAD_CPUTIME_ID);
    void *result = __libc_malloc(size);
    int saved = errno;
    uint64_t elapsed_cpu = nanos(CLOCK_THREAD_CPUTIME_ID) - cpu;
    uint64_t elapsed_wall = nanos(CLOCK_MONOTONIC) - wall;
    malloc_calls++;
    if (elapsed_wall >= 500000) report("malloc", size, elapsed_wall, elapsed_cpu);
    errno = saved;
    return result;
}

__attribute__((constructor)) static void initialize(void) {
    real_opendir = (DIR *(*)(const char *))dlsym(RTLD_NEXT, "opendir");
}

DIR *opendir(const char *path) {
    if (!real_opendir) initialize();
    if (!real_opendir) { errno = ENOSYS; return NULL; }
    const char *previous = directory_path;
    unsigned previous_calls = malloc_calls;
    directory_path = path;
    malloc_calls = 0;
    uint64_t wall = nanos(CLOCK_MONOTONIC), cpu = nanos(CLOCK_THREAD_CPUTIME_ID);
    DIR *result = real_opendir(path);
    int saved = errno;
    uint64_t elapsed_cpu = nanos(CLOCK_THREAD_CPUTIME_ID) - cpu;
    uint64_t elapsed_wall = nanos(CLOCK_MONOTONIC) - wall;
    if (elapsed_wall >= 500000) report("opendir", 0, elapsed_wall, elapsed_cpu);
    directory_path = previous;
    malloc_calls = previous_calls;
    errno = saved;
    return result;
}
