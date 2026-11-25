QCC ?= qcc
ARCH ?= gcc_ntox86_64
CFLAGS ?= -V$(ARCH) -O2 -Wall -Wextra

all: motor_ctrl_buggy motor_ctrl_fixed latency_bench

motor_ctrl_buggy: motor_ctrl.c
	$(QCC) $(CFLAGS) motor_ctrl.c -lslog2 -o $@

motor_ctrl_fixed: motor_ctrl.c
	$(QCC) $(CFLAGS) -DFIX_PRIO_INHERIT motor_ctrl.c -lslog2 -o $@

latency_bench: latency_bench.c
	$(QCC) $(CFLAGS) latency_bench.c -o $@

deploy-qemu: all
	ssh -p 2222 qnxuser@127.0.0.1 "mkdir -p /tmp/qnx-mcp-staging"
	scp -P 2222 motor_ctrl_buggy motor_ctrl_fixed latency_bench qnxuser@127.0.0.1:/tmp/qnx-mcp-staging/

clean:
	rm -f motor_ctrl_buggy motor_ctrl_fixed latency_bench
