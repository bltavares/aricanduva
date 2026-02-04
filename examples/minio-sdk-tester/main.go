package main

import (
	"bufio"
	"context"
	"log"
	"mime"
	"net/url"
	"os"
	"path"
	"time"

	"github.com/minio/minio-go/v7"
	"github.com/minio/minio-go/v7/pkg/credentials"

	"codeberg.org/gruf/go-storage/s3"
)

const (
	urlCacheTTL = time.Hour * 24
)

func main() {
	endpoint := "localhost:3000"
	accessKeyID := "banana"
	secretAccessKey := "bananabanana"
	useSSL := false
	bucket := "banana-bucket"
	key := "path/to/README.md"

	ctx := context.Background()

	minioGoTest(ctx, endpoint, accessKeyID, secretAccessKey, useSSL, bucket, key)

	storageGoTest(ctx, endpoint, bucket, accessKeyID, secretAccessKey, useSSL, key)

}

func storageGoTest(ctx context.Context, endpoint string, bucket string, accessKeyID string, secretAccessKey string, useSSL bool, key string) {
	log.Print("Starting to test go-storage")

	var objCache s3.EntryCache
	storage, err := s3.Open(endpoint, bucket, &s3.Config{
		CoreOpts: minio.Options{
			Creds:  credentials.NewStaticV4(accessKeyID, secretAccessKey, ""),
			Secure: useSSL,
		},
		PutChunkSize: 5 * 1024 * 1024, // 5MiB
		ListSize:     200,
		Cache:        objCache,
	})

	// Open file at path for reading.
	file, err := os.Open("../../README.md")
	defer file.Close()
	if err != nil {
		log.Fatalln(err)
	}

	info, err := storage.PutObject(ctx, key, file, minio.PutObjectOptions{
		ContentType: "text/markdown",
	})
	if err != nil {
		log.Fatalln(err)
	}

	log.Printf("Uploaded %v", info)

	u, err := storage.Client().PresignedGetObject(ctx, bucket, key, urlCacheTTL, url.Values{
		"response-content-type": []string{mime.TypeByExtension(path.Ext(key))},
	})
	if err != nil {
		log.Fatalln(err)
	}
	log.Printf("Signed %v", u)

	err = storage.Client().RemoveObject(ctx, bucket, key, minio.RemoveObjectOptions{})
	if err != nil {
		log.Fatalln(err)
	}
	log.Printf("Deleted")

}

func minioGoTest(ctx context.Context, endpoint string, accessKeyID string, secretAccessKey string, useSSL bool, bucket string, key string) {
	log.Print("Starting to test minio-go")

	// Initialize minio client object.
	minioClient, err := minio.NewCore(endpoint, &minio.Options{
		Creds:  credentials.NewStaticV4(accessKeyID, secretAccessKey, ""),
		Secure: useSSL,
	})
	if err != nil {
		log.Fatalln(err)
	}

	response, err := minioClient.BucketExists(ctx, bucket)
	if err != nil {
		log.Fatalln(err)
	}
	log.Printf("Bucket exists: %v", response)

	// Open file at path for reading.
	file, err := os.Open("../../README.md")
	defer file.Close()
	if err != nil {
		log.Fatalln(err)
	}
	info, err := file.Stat()
	if err != nil {
		log.Fatalln(err)
	}
	reader := bufio.NewReader(file)

	u, err := minioClient.PutObject(ctx, bucket, key, reader, info.Size(), "", "", minio.PutObjectOptions{})
	if err != nil {
		log.Fatalln(err)
	}
	log.Printf("Upload %v", u)

	r, err := minioClient.StatObject(ctx, bucket, key, minio.StatObjectOptions{})
	if err != nil {
		log.Fatalln(err)
	}

	log.Printf("File %v", r)

	err = minioClient.RemoveObject(ctx, bucket, key, minio.RemoveObjectOptions{})
	if err != nil {
		log.Fatalln(err)
	}
}
