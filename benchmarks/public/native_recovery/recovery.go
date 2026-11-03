package main

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"time"

	warm "github.com/writerslogic/warm/bindings/go"
)

func check(condition bool, err error) {
	if err != nil || !condition {
		panic(fmt.Sprintf("recovery assertion failed: %v", err))
	}
}

func main() {
	check(len(os.Args) == 3 && (os.Args[1] == "write" || os.Args[1] == "verify"), nil)
	store, err := warm.Open(os.Args[2], 1024)
	check(true, err)
	defer store.Close()
	ctx := context.Background()
	if os.Args[1] == "write" {
		for _, stage := range []struct {
			count, revision int
			word, phase     string
		}{{32, 1, "original", "inserted"}, {16, 2, "replacement", "replaced"}} {
			documents := make([]warm.Document, stage.count)
			for index := range documents {
				documents[index] = warm.Document{ID: fmt.Sprintf("r%02d", index), Text: fmt.Sprintf("recovery record %02d %s", index, stage.word), Metadata: map[string]int{"revision": stage.revision}}
			}
			chunks, err := store.Ingest(ctx, documents...)
			check(len(chunks) == stage.count, err)
			for _, count := range chunks {
				check(count == 1, nil)
			}
			check(true, store.Flush(ctx))
			fmt.Printf("{\"phase\":%q,\"documents\":%d}\n", stage.phase, stage.count)
		}
		for index := 16; index < 24; index++ {
			deleted, err := store.Delete(ctx, fmt.Sprintf("r%02d", index))
			check(deleted, err)
		}
		check(true, store.Flush(ctx))
		fmt.Println("{\"phase\":\"ready\",\"deleted\":8}")
		for {
			time.Sleep(time.Hour)
		}
	}
	for index := 0; index < 32; index++ {
		id := fmt.Sprintf("r%02d", index)
		hits, err := store.Search(ctx, "recovery", &warm.SearchOptions{K: 1, DocumentIDs: []string{id}})
		check(true, err)
		if index >= 16 && index < 24 {
			check(len(hits) == 0, nil)
			continue
		}
		check(len(hits) == 1, nil)
		word, revision := "original", 1
		if index < 16 {
			word, revision = "replacement", 2
		}
		check(hits[0].DocumentID == id && hits[0].Text != nil && *hits[0].Text == fmt.Sprintf("recovery record %02d %s", index, word), nil)
		var metadata map[string]int
		check(true, json.Unmarshal(hits[0].Metadata, &metadata))
		check(metadata["revision"] == revision && len(metadata) == 1, nil)
	}
	chunks, err := store.Ingest(ctx, warm.Document{ID: "after", Text: "recovery resumed write"})
	check(len(chunks) == 1 && chunks[0] == 1, err)
	check(true, store.Flush(ctx))
	check(true, store.Close())
	store, err = warm.Open(os.Args[2], 1024)
	check(true, err)
	hits, err := store.Search(ctx, "recovery", &warm.SearchOptions{DocumentIDs: []string{"after"}})
	check(len(hits) == 1, err)
	check(hits[0].DocumentID == "after" && hits[0].Text != nil && *hits[0].Text == "recovery resumed write", nil)
	check(true, store.Close())
	fmt.Println("{\"phase\":\"verified\",\"records\":24,\"deleted\":8,\"resumedWrite\":true}")
}
