run-benchmark:
	(trap 'kill 0' SIGINT && \
	printf "avg_wait 0 \n reliability 1 \n start v4 \n start v6 \n list \n loop " | cargo run --bin example-server & \
	sleep 1 && cargo bench)
