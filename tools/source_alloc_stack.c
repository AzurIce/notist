/* Diagnostic glibc API samples; no allocator replacement. Target executable only.
 * Backtraces are probability samples, not exact per-site allocation counts.
 * Exact scoped Rust counts are collected by a separate diagnostic executable. */
#define _GNU_SOURCE
#include <stdatomic.h>
#include <stdint.h>
#include <stddef.h>
#include <stdlib.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <execinfo.h>
#include <malloc.h>
extern void *__libc_malloc(size_t);
extern void *__libc_calloc(size_t,size_t);
extern void *__libc_realloc(void*,size_t);
extern void __libc_free(void*);
static int fd=-1;
static _Atomic int active;
static _Atomic uint64_t events[4], bytes[4], dropped;
static _Thread_local int busy;
static _Thread_local uint64_t rng;
struct record { uint64_t kind, size, old_usable, depth; void *stack[24]; };
static int sample(unsigned kind) {
    if (!rng) rng=(uintptr_t)&rng ^ (uint64_t)gettid() ^ UINT64_C(0x71ab3828a11df);
    rng^=rng<<13; rng^=rng>>7; rng^=rng<<17;
    return !(rng & (kind==2?255:2047));
}
static void record(unsigned kind,size_t size,size_t old) {
    struct record r={.kind=kind,.size=size,.old_usable=old};
    r.depth=backtrace(r.stack,24);
    if (write(fd,&r,sizeof(r))!=(ssize_t)sizeof(r)) atomic_fetch_add(&dropped,1);
}
void *malloc(size_t n) { void *p=__libc_malloc(n); if(p && atomic_load(&active) && !busy) { busy=1; atomic_fetch_add(&events[0],1); atomic_fetch_add(&bytes[0],n); if(sample(0))record(0,n,0); busy=0; } return p; }
void *calloc(size_t a,size_t b) { void *p=__libc_calloc(a,b); if(p && atomic_load(&active) && !busy) { busy=1; atomic_fetch_add(&events[1],1); atomic_fetch_add(&bytes[1],a*b); if(sample(1))record(1,a*b,0); busy=0; } return p; }
void *realloc(void *p,size_t n) { int on=atomic_load(&active) && !busy; int take=0; size_t old=0; if(on) { busy=1; take=sample(2); if(take && p)old=malloc_usable_size(p); } void *q=__libc_realloc(p,n); if(on) { if(q) { atomic_fetch_add(&events[2],1); atomic_fetch_add(&bytes[2],n); if(take)record(2,n,old); } busy=0; } return q; }
void free(void *p) { if(p && atomic_load(&active) && !busy)atomic_fetch_add(&events[3],1); __libc_free(p); }
__attribute__((constructor)) static void start(void) {
    char exe[4096]; const char *target=getenv("SOURCE_ALLOC_TARGET"), *path=getenv("SOURCE_ALLOC_STACKS");
    ssize_t n=readlink("/proc/self/exe",exe,sizeof(exe)-1); if(n<0 || !target || !path)return; exe[n]=0; if(strcmp(target,exe))return;
    char maps[4096]; snprintf(maps,sizeof(maps),"%s.maps",path); int in=open("/proc/self/maps",O_RDONLY),out=open(maps,O_WRONLY|O_CREAT|O_EXCL,0600);
    char block[8192]; ssize_t r; if(in>=0 && out>=0) { while((r=read(in,block,sizeof(block)))>0)write(out,block,r); } if(in>=0)close(in); if(out>=0)close(out);
    fd=open(path,O_WRONLY|O_CREAT|O_EXCL|O_APPEND,0600); if(fd>=0)atomic_store(&active,1);
}
__attribute__((destructor)) static void finish(void) {
    if(!atomic_exchange(&active,0))return;
    fprintf(stderr,"[source-alloc-stack] record_size=%zu malloc=%llu calloc=%llu realloc=%llu free=%llu malloc_bytes=%llu calloc_bytes=%llu realloc_bytes=%llu dropped=%llu\n",sizeof(struct record),(unsigned long long)events[0],(unsigned long long)events[1],(unsigned long long)events[2],(unsigned long long)events[3],(unsigned long long)bytes[0],(unsigned long long)bytes[1],(unsigned long long)bytes[2],(unsigned long long)dropped);
    close(fd);
}
