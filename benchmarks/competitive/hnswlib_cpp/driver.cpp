// Copyright 2024-2026 WritersLogic Contributors
// SPDX-License-Identifier: Apache-2.0
//
// Native C++ hnswlib serving driver for the `hnswlib-cpp` system of warm-bench.
//
// It loads the index file the Python hnswlib build saved (the on-disk format is shared), then
// answers line commands on stdin so ann_worker.py can run the same recall pass and timed passes it
// runs for every other system, with the search itself outside Python:
//   ids  <ef> <out>   untimed pass over every query, ids written as int32 [nq, 10]
//   time <ef> <out>   one timed pass, per-query nanoseconds written as int64 [nq]
//   warm <n>          run the first n queries untimed
//   rss               print resident bytes
//   quit
// The search call and result drain mirror hnswlib's Python binding (python_bindings/bindings.cpp,
// knnQuery_return_numpy with num_threads = 1): searchKnn(query, k, nullptr), then the queue is popped
// into a contiguous row nearest first. Each query is timed with steady_clock (CLOCK_MONOTONIC) around
// that whole sequence, as perf_counter_ns is around the Python call.
//
//   driver serve <index.bin> <queries.f32> <nq> <dim> <l2|ip> <max_elements>
//   driver trivial <dim> <calls>   one-vector index, k=1, ef=1: per-call searchKnn time in ns
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <fstream>
#include <iostream>
#include <memory>
#include <random>
#include <sstream>
#include <stdexcept>
#include <string>
#include <vector>

#include "hnswlib/hnswlib.h"

namespace {
constexpr size_t K = 10;
using Clock = std::chrono::steady_clock;

struct Search {
    hnswlib::HierarchicalNSW<float>* index;
    size_t k;
    // Mirrors the binding: search, then pop the queue into row[k-1 .. 0].
    void run(const float* q, hnswlib::labeltype* out) const {
        auto result = index->searchKnn(static_cast<const void*>(q), k, nullptr);
        if (result.size() != k) throw std::runtime_error("short result");
        for (size_t i = k; i-- > 0;) {
            out[i] = result.top().second;
            result.pop();
        }
    }
};

uint64_t rss_bytes() {
    std::ifstream f("/proc/self/statm");
    uint64_t size = 0, resident = 0;
    f >> size >> resident;
    return resident * 4096;
}

std::vector<float> read_f32(const std::string& path, size_t count) {
    std::vector<float> v(count);
    std::ifstream f(path, std::ios::binary);
    f.read(reinterpret_cast<char*>(v.data()), static_cast<std::streamsize>(count * sizeof(float)));
    if (!f) throw std::runtime_error("short read: " + path);
    return v;
}

int serve(int argc, char** argv) {
    if (argc != 8) throw std::runtime_error("serve needs 6 arguments");
    const std::string index_path = argv[2], query_path = argv[3];
    const size_t nq = std::stoull(argv[4]), dim = std::stoull(argv[5]);
    const std::string space_name = argv[6];
    const size_t max_elements = std::stoull(argv[7]);
    std::unique_ptr<hnswlib::SpaceInterface<float>> space;
    if (space_name == "l2") space.reset(new hnswlib::L2Space(dim));
    else if (space_name == "ip") space.reset(new hnswlib::InnerProductSpace(dim));
    else throw std::runtime_error("space must be l2 or ip");
    const std::vector<float> queries = read_f32(query_path, nq * dim);

    const auto t_open = Clock::now();
    hnswlib::HierarchicalNSW<float> index(space.get(), index_path, false, max_elements, false);
    const double open_s = std::chrono::duration<double>(Clock::now() - t_open).count();
    Search search{&index, K};
    std::vector<hnswlib::labeltype> row(K);
    index.setEf(K);
    search.run(queries.data(), row.data());
    std::printf("READY open_s=%.9f rss=%llu\n", open_s, static_cast<unsigned long long>(rss_bytes()));
    std::fflush(stdout);

    std::string line;
    while (std::getline(std::cin, line)) {
        std::istringstream in(line);
        std::string cmd;
        in >> cmd;
        if (cmd == "quit") break;
        if (cmd == "rss") {
            std::printf("OK %llu\n", static_cast<unsigned long long>(rss_bytes()));
        } else if (cmd == "warm") {
            size_t n = 0;
            in >> n;
            for (size_t i = 0; i < n && i < nq; i++) search.run(queries.data() + i * dim, row.data());
            std::printf("OK\n");
        } else if (cmd == "ids" || cmd == "time") {
            size_t ef = 0;
            std::string out;
            in >> ef >> out;
            index.setEf(ef);
            if (cmd == "ids") {
                std::vector<int32_t> ids(nq * K);
                for (size_t i = 0; i < nq; i++) {
                    search.run(queries.data() + i * dim, row.data());
                    for (size_t j = 0; j < K; j++) ids[i * K + j] = static_cast<int32_t>(row[j]);
                }
                std::ofstream(out, std::ios::binary).write(reinterpret_cast<const char*>(ids.data()), static_cast<std::streamsize>(ids.size() * 4));
            } else {
                std::vector<int64_t> lat(nq);
                for (size_t i = 0; i < nq; i++) {
                    const float* q = queries.data() + i * dim;
                    const auto t = Clock::now();
                    search.run(q, row.data());
                    lat[i] = std::chrono::duration_cast<std::chrono::nanoseconds>(Clock::now() - t).count();
                }
                std::ofstream(out, std::ios::binary).write(reinterpret_cast<const char*>(lat.data()), static_cast<std::streamsize>(lat.size() * 8));
            }
            std::printf("OK\n");
        } else {
            std::printf("ERR unknown command\n");
        }
        std::fflush(stdout);
    }
    return 0;
}

// One 128-d vector, k=1, ef=1: the search is a single distance computation, so the per-call time is
// the call overhead floor. Same index shape as the Python overhead script builds.
int trivial(int argc, char** argv) {
    if (argc != 4) throw std::runtime_error("trivial needs <dim> <calls>");
    const size_t dim = std::stoull(argv[2]), calls = std::stoull(argv[3]);
    hnswlib::L2Space space(dim);
    hnswlib::HierarchicalNSW<float> index(&space, 1, 16, 200, 100);
    std::mt19937 rng(1);
    std::normal_distribution<float> normal;
    std::vector<float> base(dim), query(dim);
    for (auto& x : base) x = normal(rng);
    for (auto& x : query) x = normal(rng);
    index.addPoint(base.data(), 0);
    index.setEf(1);
    Search search{&index, 1};
    hnswlib::labeltype out[1];
    for (size_t i = 0; i < 10000; i++) search.run(query.data(), out);
    std::vector<int64_t> lat(calls);
    for (size_t i = 0; i < calls; i++) {
        const auto t = Clock::now();
        search.run(query.data(), out);
        lat[i] = std::chrono::duration_cast<std::chrono::nanoseconds>(Clock::now() - t).count();
    }
    // Clock floor: two back-to-back reads.
    std::vector<int64_t> floor_ns(calls);
    for (size_t i = 0; i < calls; i++) {
        const auto t = Clock::now();
        floor_ns[i] = std::chrono::duration_cast<std::chrono::nanoseconds>(Clock::now() - t).count();
    }
    std::fwrite(lat.data(), 8, lat.size(), stdout);
    std::fwrite(floor_ns.data(), 8, floor_ns.size(), stdout);
    return 0;
}
}  // namespace

int main(int argc, char** argv) {
    try {
        if (argc < 2) throw std::runtime_error("usage: driver serve|trivial ...");
        const std::string mode = argv[1];
        if (mode == "serve") return serve(argc, argv);
        if (mode == "trivial") return trivial(argc, argv);
        throw std::runtime_error("unknown mode");
    } catch (const std::exception& e) {
        std::fprintf(stderr, "driver error: %s\n", e.what());
        return 2;
    }
}
