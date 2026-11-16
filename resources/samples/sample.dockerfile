# Source formatting only: no image is built or downloaded.
FROM alpine:3.22 AS base
WORKDIR /app
COPY ["config.json", "/app/config.json"]
ENV APP_MODE=production
RUN printf '%s\n' 'ready' > /app/status
EXPOSE 8080
CMD ["cat", "/app/status"]
