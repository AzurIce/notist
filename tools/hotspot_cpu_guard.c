/* User-mode sampling only for the selected executable, never its children. */
#define _GNU_SOURCE
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <sys/resource.h>
extern int ProfilerStart(const char *path);
extern void ProfilerStop(void);
static int active;
__attribute__((constructor)) static void start(void) {
    char exe[4096];
    const char *target=getenv("HOTSPOT_CPU_TARGET"), *output=getenv("HOTSPOT_CPU_OUTPUT");
    ssize_t n=readlink("/proc/self/exe",exe,sizeof(exe)-1);
    if(n<0 || !target || !output)return;
    exe[n]=0;
    if(strcmp(target,exe))return;
    if(!ProfilerStart(output)) { fprintf(stderr,"hotspot profiler failed to start\n"); _exit(125); }
    active=1;
}
__attribute__((destructor)) static void finish(void) {
    if(!active)return;
    ProfilerStop();
    struct rusage r;
    if(!getrusage(RUSAGE_SELF,&r))fprintf(stderr,"[hotspot-parent-cpu] user=%.6f system=%.6f\n",r.ru_utime.tv_sec+r.ru_utime.tv_usec/1e6,r.ru_stime.tv_sec+r.ru_stime.tv_usec/1e6);
}
