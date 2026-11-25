/*
 * QNX Neutrino SDP 8.0 Microsecond Scheduling Jitter Benchmark
 * Emits comma-separated microsecond wake-up jitter samples parsed directly by
 * `qnx-mcp-workbench` (`qnx_latency_bench`).
 *
 * Build (inside QNX SDP 8.0 environment):
 *   qcc -Vgcc_ntox86_64 -O2 -Wall latency_bench.c -o latency_bench
 */

#include <stdint.h>
#include <stdio.h>
#include <time.h>

#define SAMPLES 40
#define PERIOD_NS 1000000LL /* 1 ms periodic timer */

static int64_t ts_diff_ns(const struct timespec *a, const struct timespec *b) {
    return (int64_t)(a->tv_sec - b->tv_sec) * 1000000000LL + (int64_t)(a->tv_nsec - b->tv_nsec);
}

int main(void) {
    struct timespec next, actual;
    clock_gettime(CLOCK_MONOTONIC, &next);

    for (int i = 0; i < SAMPLES; i++) {
        next.tv_nsec += PERIOD_NS;
        if (next.tv_nsec >= 1000000000LL) {
            next.tv_sec += 1;
            next.tv_nsec -= 1000000000LL;
        }
        clock_nanosleep(CLOCK_MONOTONIC, TIMER_ABSTIME, &next, NULL);
        clock_gettime(CLOCK_MONOTONIC, &actual);

        int64_t delta_ns = ts_diff_ns(&actual, &next);
        if (delta_ns < 0) delta_ns = 0;
        double jitter_us = (double)delta_ns / 1000.0;

        if (i > 0) printf(",");
        printf("%.1f", jitter_us);
    }
    printf("\n");
    return 0;
}
