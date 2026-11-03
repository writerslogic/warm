#include "writerslogic_warm.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static warm_str_t string(const char *value) {
  return (warm_str_t){(const uint8_t *)value, strlen(value)};
}

static void check(int condition) {
  if (!condition) {
    fprintf(stderr, "recovery assertion failed: %s\n", hms1_last_error_message());
    exit(1);
  }
}

static int equal(warm_str_t actual, const char *expected) {
  return actual.ptr && actual.len == strlen(expected) &&
         memcmp(actual.ptr, expected, actual.len) == 0;
}

static void ingest(warm_store_t *store, int count, int revision) {
  warm_document_t documents[32] = {0};
  uint32_t chunks[32] = {0};
  char ids[32][16], texts[32][64];
  for (int index = 0; index < count; index++) {
    snprintf(ids[index], sizeof ids[index], "r%02d", index);
    snprintf(texts[index], sizeof texts[index], "recovery record %02d %s", index,
             revision == 1 ? "original" : "replacement");
    documents[index].id = string(ids[index]);
    documents[index].text = string(texts[index]);
    documents[index].metadata_json = string(revision == 1 ? "{\"revision\":1}" : "{\"revision\":2}");
  }
  check(hms1_store_ingest(store, documents, (size_t)count, chunks) == WARM_OK);
  for (int index = 0; index < count; index++) check(chunks[index] == 1);
  check(hms1_store_flush(store) == WARM_OK);
}

int main(int argc, char **argv) {
  check(argc == 3 && (strcmp(argv[1], "write") == 0 || strcmp(argv[1], "verify") == 0));
  warm_store_t *store = NULL;
  check(hms1_store_open(string(argv[2]), 1024, NULL, &store) == WARM_OK);
  if (strcmp(argv[1], "write") == 0) {
    ingest(store, 32, 1);
    puts("{\"phase\":\"inserted\",\"documents\":32}");
    fflush(stdout);
    ingest(store, 16, 2);
    puts("{\"phase\":\"replaced\",\"documents\":16}");
    fflush(stdout);
    for (int index = 16; index < 24; index++) {
      char id[16];
      snprintf(id, sizeof id, "r%02d", index);
      int32_t deleted = 0;
      check(hms1_store_delete(store, string(id), &deleted) == WARM_OK && deleted == 1);
    }
    check(hms1_store_flush(store) == WARM_OK);
    puts("{\"phase\":\"ready\",\"deleted\":8}");
    fflush(stdout);
    for (;;) pause();
  }
  for (int index = 0; index < 32; index++) {
    char id[16], expected[64];
    snprintf(id, sizeof id, "r%02d", index);
    snprintf(expected, sizeof expected, "recovery record %02d %s", index,
             index < 16 ? "replacement" : "original");
    warm_str_t identifier = string(id);
    warm_search_options_t options;
    hms1_search_options_init(&options);
    options.document_ids = &identifier;
    options.document_id_count = 1;
    options.k = 1;
    warm_results_t *results = NULL;
    check(hms1_store_search(store, string("recovery"), &options, &results) == WARM_OK);
    if (index >= 16 && index < 24) {
      check(hms1_results_count(results) == 0);
    } else {
      check(hms1_results_count(results) == 1);
      warm_result_t result = {0};
      check(hms1_results_get(results, 0, &result) == WARM_OK);
      check(equal(result.document_id, id) && equal(result.text, expected));
      check(equal(result.metadata_json, index < 16 ? "{\"revision\":2}" : "{\"revision\":1}"));
    }
    hms1_results_free(results);
  }
  warm_document_t resumed = {0};
  resumed.id = string("after");
  resumed.text = string("recovery resumed write");
  uint32_t chunks = 0;
  check(hms1_store_ingest(store, &resumed, 1, &chunks) == WARM_OK && chunks == 1);
  check(hms1_store_flush(store) == WARM_OK);
  check(hms1_store_close(store) == WARM_OK);
  check(hms1_store_open(string(argv[2]), 1024, NULL, &store) == WARM_OK);
  warm_str_t identifier = string("after");
  warm_search_options_t options;
  hms1_search_options_init(&options);
  options.document_ids = &identifier;
  options.document_id_count = 1;
  warm_results_t *results = NULL;
  check(hms1_store_search(store, string("recovery"), &options, &results) == WARM_OK);
  check(hms1_results_count(results) == 1);
  warm_result_t result = {0};
  check(hms1_results_get(results, 0, &result) == WARM_OK);
  check(equal(result.document_id, "after") && equal(result.text, "recovery resumed write"));
  hms1_results_free(results);
  check(hms1_store_close(store) == WARM_OK);
  puts("{\"phase\":\"verified\",\"records\":24,\"deleted\":8,\"resumedWrite\":true}");
  return 0;
}
