import com.writerslogic.warm.Document;
import com.writerslogic.warm.SearchOptions;
import com.writerslogic.warm.Store;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

public final class RecoveryConsumer {
    private static void check(boolean condition) {
        if (!condition) throw new IllegalStateException("recovery assertion failed");
    }

    public static void main(String[] args) throws Exception {
        check(args.length == 2 && (args[0].equals("write") || args[0].equals("verify")));
        try (var store = Store.open(Path.of(args[1]), 1024)) {
            if (args[0].equals("write")) {
                for (int revision = 1; revision <= 2; revision++) {
                    int count = revision == 1 ? 32 : 16;
                    String word = revision == 1 ? "original" : "replacement";
                    var documents = new ArrayList<Document>();
                    for (int index = 0; index < count; index++) {
                        documents.add(Document.of("r%02d".formatted(index), "recovery record %02d %s".formatted(index, word))
                                .withMetadataJson("{\"revision\":%d}".formatted(revision)));
                    }
                    int[] chunks = store.ingest(documents);
                    check(chunks.length == count);
                    for (int value : chunks) check(value == 1);
                    store.flush();
                    System.out.println("{\"phase\":\"%s\",\"documents\":%d}".formatted(revision == 1 ? "inserted" : "replaced", count));
                }
                for (int index = 16; index < 24; index++) check(store.delete("r%02d".formatted(index)));
                store.flush();
                System.out.println("{\"phase\":\"ready\",\"deleted\":8}");
                System.out.flush();
                Thread.sleep(Long.MAX_VALUE);
                throw new IllegalStateException("writer unexpectedly resumed");
            }
            for (int index = 0; index < 32; index++) {
                String id = "r%02d".formatted(index);
                var hits = store.search("recovery", SearchOptions.builder().k(1).documentIds(List.of(id)).build());
                if (index >= 16 && index < 24) {
                    check(hits.isEmpty());
                } else {
                    check(hits.size() == 1);
                    var hit = hits.getFirst();
                    check(hit.documentId().equals(id));
                    check(hit.text().equals("recovery record %02d %s".formatted(index, index < 16 ? "replacement" : "original")));
                    check(hit.metadataJson().equals(index < 16 ? "{\"revision\":2}" : "{\"revision\":1}"));
                }
            }
            check(store.ingest(Document.of("after", "recovery resumed write")) == 1);
            store.flush();
        }
        try (var store = Store.open(Path.of(args[1]), 1024)) {
            var hits = store.search("recovery", SearchOptions.builder().documentIds(List.of("after")).build());
            check(hits.size() == 1 && hits.getFirst().documentId().equals("after")
                    && hits.getFirst().text().equals("recovery resumed write"));
        }
        System.out.println("{\"phase\":\"verified\",\"records\":24,\"deleted\":8,\"resumedWrite\":true}");
    }
}
