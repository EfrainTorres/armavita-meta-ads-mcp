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

# requirements.txt is `-e .`, so the project metadata (pyproject.toml, README.md)
# and source must be present BEFORE the install runs, or the editable build fails.
COPY pyproject.toml README.md ./
COPY src ./src
COPY scripts ./scripts
COPY requirements.txt ./requirements.txt
RUN uv pip install --system --requirement requirements.txt

ENV PYTHONPATH=/opt/armavita/src

# Run as an unprivileged user rather than root.
RUN useradd --create-home --uid 10001 armavita
USER armavita

ENTRYPOINT ["python", "-m", "armavita_meta_ads_mcp"]
