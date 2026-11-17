/* Copyright 2024-2026 WritersLogic Contributors */
/* SPDX-License-Identifier: Apache-2.0 */

/* Open a store, ingest two documents, search, free. docs/runtimes/c/README.md shows this file;
   bindings/c/tests/c/run.sh compiles and runs it. Usage: quickstart STORE_DIR */

#include <stdio.h>
#include <string.h>

#include "writerslogic_warm.h"

static warm_str_t str(const char *s) { return (warm_str_t){(const uint8_t *)s, strlen(s)}; }

static int fail(const char *what) {
  fprintf(stderr, "%s failed (%d): %s\n", what, warm_last_error_code(), warm_last_error_message());
  return 1;
}

int main(int argc, char **argv) {
  if (argc != 2) {
    fprintf(stderr, "usage: %s STORE_DIR\n", argv[0]);
    return 2;
  }

  warm_store_t *store = NULL;
  if (warm_store_open(str(argv[1]), 2048, NULL, &store) != WARM_OK) return fail("open");

  warm_document_t docs[2];
  memset(docs, 0, sizeof docs);
  docs[0].id = str("harbor");
  docs[0].text = str("The harbor opens at six in the morning.");
  docs[1].id = str("market");
  docs[1].text = str("The market closes at noon on Sundays.");
  if (warm_store_ingest(store, docs, 2, NULL) != WARM_OK) {
    fail("ingest");
    warm_store_close(store);
    return 1;
  }

  warm_search_options_t options;
  warm_search_options_init(&options);
  options.k = 1;
  warm_results_t *results = NULL;
  if (warm_store_search(store, str("when does the harbor open"), &options, &results) != WARM_OK) {
    fail("search");
    warm_store_close(store);
    return 1;
  }

  for (size_t i = 0; i < warm_results_count(results); i++) {
    warm_result_t hit;
    if (warm_results_get(results, i, &hit) != WARM_OK) continue;
    /* Strings are length-prefixed and not NUL terminated. */
    printf("%.*s %.3f\n", (int)hit.document_id.len, (const char *)hit.document_id.ptr, hit.score);
  }

  warm_results_free(results);
  return warm_store_close(store) == WARM_OK ? 0 : fail("close");
}
