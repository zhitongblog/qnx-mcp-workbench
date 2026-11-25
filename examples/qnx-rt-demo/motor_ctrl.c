/*
 * QNX Neutrino SDP 8.0 Real-Time Motor Control Demo
 * Demonstrates a classic priority inversion on a shared CAN bus mutex and its
 * deterministic fix using PTHREAD_PRIO_INHERIT, emitting structured slog2 logs.
 *
 * Build (inside QNX SDP 8.0 environment):
 *   qcc -Vgcc_ntox86_64 -O2 -Wall motor_ctrl.c -lslog2 -o motor_ctrl_buggy
 *   qcc -Vgcc_ntox86_64 -O2 -Wall -DFIX_PRIO_INHERIT motor_ctrl.c -lslog2 -o motor_ctrl_fixed
 */

#include <errno.h>
#include <pthread.h>
#include <sched.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>
#include <sys/slog2.h>

static pthread_mutex_t g_can_bus_lock;
static slog2_buffer_t g_slog_buf = NULL;
static volatile int g_running = 1;

static uint64_t now_us(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (uint64_t)ts.tv_sec * 1000000ULL + (uint64_t)(ts.tv_nsec / 1000ULL);
}

static void init_mutex(void) {
    pthread_mutexattr_t attr;
    pthread_mutexattr_init(&attr);
#ifdef FIX_PRIO_INHERIT
    /* QNX Neutrino priority inheritance prevents unbounded priority inversion */
    pthread_mutexattr_setprotocol(&attr, PTHREAD_PRIO_INHERIT);
#else
    pthread_mutexattr_setprotocol(&attr, PTHREAD_PRIO_NONE);
#endif
    pthread_mutex_init(&g_can_bus_lock, &attr);
    pthread_mutexattr_destroy(&attr);
}

/* Low-priority telemetry logger (prio 10) holding shared CAN mutex */
static void *low_prio_telemetry_worker(void *arg) {
    (void)arg;
    while (g_running) {
        pthread_mutex_lock(&g_can_bus_lock);
        /* Simulate slow CAN frame serialization */
        usleep(4000);
        pthread_mutex_unlock(&g_can_bus_lock);
        usleep(10000);
    }
    return NULL;
}

/* Medium-priority CPU-bound worker (prio 18) preempting low-prio lock holder */
static void *medium_prio_noise_worker(void *arg) {
    (void)arg;
    volatile uint64_t sink = 0;
    while (g_running) {
        for (int i = 0; i < 400000; i++) {
            sink += (uint64_t)i;
        }
        usleep(5000);
    }
    return NULL;
}

/* High-priority 1000Hz motor control loop (prio 25 FIFO) */
static void *high_prio_motor_loop(void *arg) {
    (void)arg;
    for (int iter = 0; iter < 50 && g_running; iter++) {
        uint64_t t0 = now_us();
        pthread_mutex_lock(&g_can_bus_lock);
        uint64_t wait_us = now_us() - t0;
        pthread_mutex_unlock(&g_can_bus_lock);

        if (wait_us > 10000) {
            slog2f(g_slog_buf, 1001, SLOG2_CRITICAL,
                   "watchdog timeout expired on motor_ctrl (delta=%.2fms)",
                   (double)wait_us / 1000.0);
        } else if (wait_us > 2000) {
            slog2f(g_slog_buf, 1002, SLOG2_ERROR,
                   "mutex stall detected: priority inversion on can_bus_lock (wait=%lluus)",
                   (unsigned long long)wait_us);
        }
        usleep(1000);
    }
    g_running = 0;
    return NULL;
}

static int spawn_rt_thread(pthread_t *tid, int policy, int prio, void *(*fn)(void *)) {
    pthread_attr_t attr;
    struct sched_param sp;
    pthread_attr_init(&attr);
    pthread_attr_setinheritsched(&attr, PTHREAD_EXPLICIT_SCHED);
    pthread_attr_setschedpolicy(&attr, policy);
    memset(&sp, 0, sizeof(sp));
    sp.sched_priority = prio;
    pthread_attr_setschedparam(&attr, &sp);
    int rc = pthread_create(tid, &attr, fn, NULL);
    pthread_attr_destroy(&attr);
    return rc;
}

int main(void) {
    slog2_buffer_set_config_t cfg;
    memset(&cfg, 0, sizeof(cfg));
    cfg.buffer_set_name = "motor_ctrl";
    cfg.num_buffers = 1;
    cfg.verbosity_level = SLOG2_INFO;
    cfg.buffer_config[0].buffer_name = "rt_loop";
    cfg.buffer_config[0].num_pages = 2;

    if (slog2_register(&cfg, &g_slog_buf, 0) == -1) {
        fprintf(stderr, "slog2_register failed\n");
        return 1;
    }

    init_mutex();
    slog2f(g_slog_buf, 1000, SLOG2_INFO,
           "loop initialized at 1000Hz (prio_inherit=%s)",
#ifdef FIX_PRIO_INHERIT
           "enabled"
#else
           "disabled"
#endif
    );

    pthread_t t_low, t_med, t_high;
    spawn_rt_thread(&t_low, SCHED_RR, 10, low_prio_telemetry_worker);
    spawn_rt_thread(&t_med, SCHED_RR, 18, medium_prio_noise_worker);
    spawn_rt_thread(&t_high, SCHED_FIFO, 25, high_prio_motor_loop);

    pthread_join(t_high, NULL);
    pthread_join(t_med, NULL);
    pthread_join(t_low, NULL);
    pthread_mutex_destroy(&g_can_bus_lock);
    return 0;
}
