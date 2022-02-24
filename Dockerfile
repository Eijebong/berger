FROM rust:1.58 as builder

WORKDIR build
ADD . ./
RUN  cargo build --release

FROM debian:buster-slim
ARG APP=/app

RUN apt-get update \
    && apt-get install -y ca-certificates tzdata libpq-dev \
    && rm -rf /var/lib/apt/lists/*

EXPOSE 10000

ENV TZ=Etc/UTC \
    APP_USER=appuser

RUN groupadd $APP_USER \
    && useradd -g $APP_USER $APP_USER \
    && mkdir -p ${APP}

COPY --from=builder /build/target/release/berger ${APP}/berger
COPY --from=builder /build/templates ${APP}/templates
COPY --from=builder /build/static ${APP}/static

RUN chown -R $APP_USER:$APP_USER ${APP}

USER $APP_USER
WORKDIR ${APP}

CMD ["./berger"]
