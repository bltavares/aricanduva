# Sample environment


## CLI upload test

```sh
docker compose up -d
curl -v aricanduva.localhost:3000/healthz

export AWS_ACCESS_KEY_ID='banana'
export AWS_SECRET_ACCESS_KEY='bananabanana'
export AWS_ENDPOINT_URL_S3="http://localhost:3000"

aws s3 cp README.md s3://banana-bucket/path/to/README.md
aws s3 cp s3://banana-bucket/path/to/README.md -
```

## Service integration test

```sh
docker compose up -d

docker compose exec gotosocial sh
# ./gotosocial admin account create --email "hello@example.com" --password "bananabanana123456" --username "hello"
# ./gotosocial admin account promote --username "hello"
# exit
docker compose logs --follow
```

Then visit <http://aricanduva.localhost:3000/healthz> and <http://gotosocial.localhost/settings> to test file upload (eg: profile picture/banner), and viewing with <http://gotosocial.localhost/@hello>