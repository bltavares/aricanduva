# Integrations

## SDKs tested

:green_circle: Operations work

- [minio/minio-go](https://github.com/minio/minio-go)
- [gruf/go-storage](https://codeberg.org/gruf/go-storage)

## AWS S3 cli

:green_circle: all operations work

Setup

```sh
export AWS_ACCESS_KEY_ID='banana'
export AWS_SECRET_ACCESS_KEY='bananabanana'
export AWS_ENDPOINT_URL_S3="http://localhost:3000"
```

File operations:

```sh
# :ok:
aws s3 cp ./README.md s3://banana-bucket/path/to/README.md -

# :ok:
aws s3 cp s3://banana-bucket/path/to/README.md -

# :ok:
aws s3 presign s3://banana-bucket/path/to/README.md

# :ok:
aws s3 rm s3://banana-bucket/path/to/README.md
```

## RustFS Cli (`rc`)

:green_circle:  All file operations work

Setup


```sh
rc alias set test-server http://localhost:3000 --debug banana bananabanana
```

File operations:

```sh
# :ok:
rc cp README.md test-server/banana-bucket/path/to/README.md --debug

# :ok:
rc cat test-server/banana-bucket/path/to/README.md --debug

# :ok:
rc share test-server/banana-bucket/path/to/README.md --debug

# :ok:
rc rm test-server/banana-bucket/path/to/README.md --debug
```


## MiniIO CLI (`mc`)

:green_circle: all operations work

Setup once the access:

```sh
mc alias set test-server http://localhost:3000  --debug banana bananabanana --path on
```

File operations:

```sh
# :ok:
mc put ./README.md test-server/banana-bucket/path/to/README.md --debug

# :ok:
mc cat test-server/banana-bucket/path/to/README.md --debug
# :ok:
mc share download test-server/banana-bucket/path/to/README.md --debug

# :ok:
mc rm test-server/banana-bucket/path/to/README.md --debug 
```


## s5cmd

:yellow_circle: only rm not working

Setup

```sh
export AWS_ACCESS_KEY_ID='banana'
export AWS_SECRET_ACCESS_KEY='bananabanana'
export S3_ENDPOINT_URL="http://localhost:3000"
```

File operations:

```sh
# :ok:
s5cmd --log trace cp ./README.md s3://banana-bucket/path/to/README.md

# :ok:
s5cmd --log trace cat s3://banana-bucket/path/to/README.md

# :ok:
s5cmd --log trace presign s3://banana-bucket/path/to/README.md

# :ok:
s5cmd --log trace rm s3://banana-bucket/path/to/README.md
```