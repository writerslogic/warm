FROM warm-az1-streaming-control@sha256:ee4fd2a1b821af91813f9cf87b8d79811ac384057382b3c593ab8f7f931fe823
COPY python/ /root/.local/share/uv/python/cpython-3.11.18-linux-x86_64-gnu/
COPY venv/ /opt/warm-pr898/venv/
RUN ln -sfn /root/.local/share/uv/python/cpython-3.11.18-linux-x86_64-gnu/bin/python3.11 /opt/warm-pr898/venv/bin/python
COPY harness/ /opt/warm-pr898/harness/
COPY final-module/ /opt/warm-pr898/native/final-module/
COPY main-module/ /opt/warm-pr898/native/main-module/
COPY run-one.sh /opt/warm-pr898/run-one.sh
COPY sealed_index_run.py /opt/warm-pr898/sealed_index_run.py
COPY manifest.json /opt/warm-pr898/manifest.json
WORKDIR /opt/warm-pr898
ENTRYPOINT ["/bin/bash", "/opt/warm-pr898/run-one.sh"]
