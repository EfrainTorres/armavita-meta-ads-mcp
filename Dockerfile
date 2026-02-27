FROM python:3.11-slim AS armavita-runtime

ENV PYTHONDONTWRITEBYTECODE=1 \
    PYTHONUNBUFFERED=1 \
    PIP_DISABLE_PIP_VERSION_CHECK=1 \
    UV_SYSTEM_PYTHON=1

WORKDIR /opt/armavita

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates gcc \
    && rm -rf /var/lib/apt/lists/*

RUN python -m pip install --upgrade pip setuptools wheel \
    && python -m pip install --no-cache-dir uv

COPY requirements.txt ./requirements.txt
RUN uv pip install --system --requirement requirements.txt

COPY src ./src
COPY README.md pyproject.toml ./
COPY scripts ./scripts

ENV PYTHONPATH=/opt/armavita/src

ENTRYPOINT ["python", "-m", "armavita_meta_ads_mcp"]
