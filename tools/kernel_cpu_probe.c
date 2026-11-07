/* Disposable user-space CPU sampler gate. Only the selected Rust executable
 * starts profiling; inherited LD_PRELOAD must not profile/overwrite from Python
 * extension processes. Link against libprofiler, not an allocator replacement. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

extern int ProfilerStart(const char *);
extern void ProfilerStop(void);
static int active;

__attribute__((constructor)) static void begin_profile(void) {
    const char *target = getenv("KERNEL_CPU_TARGET");
    const char *output = getenv("KERNEL_CPU_PROFILE");
    char executable[4096];
    if (!target || !output || getenv("CPUPROFILE")) return;
    ssize_t length = readlink("/proc/self/exe", executable, sizeof(executable) - 1);
    if (length < 0) return;
    executable[length] = '\0';
    if (strcmp(target, executable)) return;
    active = ProfilerStart(output);
    fprintf(stderr, "[kernel-cpu] started=%d pid=%ld\n", active, (long)getpid());
}

__attribute__((destructor)) static void end_profile(void) {
    if (active) ProfilerStop();
}
