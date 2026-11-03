using WritersLogic.WarmMemory;

static void Check(bool condition)
{
    if (!condition) throw new InvalidOperationException("recovery assertion failed");
}

Check(args.Length == 2 && (args[0] == "write" || args[0] == "verify"));
using (var store = Store.Open(args[1], 1024))
{
    if (args[0] == "write")
    {
        for (int revision = 1; revision <= 2; revision++)
        {
            int count = revision == 1 ? 32 : 16;
            string word = revision == 1 ? "original" : "replacement";
            var documents = Enumerable.Range(0, count).Select(index =>
                new Document($"r{index:00}", $"recovery record {index:00} {word}")
                { MetadataJson = $"{{\"revision\":{revision}}}" }).ToArray();
            var chunks = store.Ingest(documents);
            Check(chunks.Length == count && chunks.All(value => value == 1));
            store.Flush();
            Console.WriteLine($"{{\"phase\":\"{(revision == 1 ? "inserted" : "replaced")}\",\"documents\":{count}}}");
        }
        for (int index = 16; index < 24; index++) Check(store.Delete($"r{index:00}"));
        store.Flush();
        Console.WriteLine("{\"phase\":\"ready\",\"deleted\":8}");
        Console.Out.Flush();
        Thread.Sleep(Timeout.Infinite);
        throw new InvalidOperationException("writer unexpectedly resumed");
    }
    for (int index = 0; index < 32; index++)
    {
        string id = $"r{index:00}";
        var hits = store.Search("recovery", new SearchOptions { K = 1, DocumentIds = new[] { id } });
        if (index >= 16 && index < 24) Check(hits.Count == 0);
        else
        {
            Check(hits.Count == 1);
            var hit = hits[0];
            Check(hit.DocumentId == id && hit.Text == $"recovery record {index:00} {(index < 16 ? "replacement" : "original")}");
            Check(hit.MetadataJson == (index < 16 ? "{\"revision\":2}" : "{\"revision\":1}"));
        }
    }
    Check(store.Ingest(new Document("after", "recovery resumed write")) == 1);
    store.Flush();
}
using (var store = Store.Open(args[1], 1024))
{
    var hits = store.Search("recovery", new SearchOptions { DocumentIds = new[] { "after" } });
    Check(hits.Count == 1 && hits[0].DocumentId == "after" && hits[0].Text == "recovery resumed write");
}
Console.WriteLine("{\"phase\":\"verified\",\"records\":24,\"deleted\":8,\"resumedWrite\":true}");
