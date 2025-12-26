-- wrk Lua script for benchmarking shape validator
-- Usage: wrk -t12 -c400 -d30s -s benchmark.lua http://localhost:8081/validate

-- Generate random shapes
local function generate_shape(id)
    return string.format([[
        {
            "id": "%s",
            "name": "Shape-%d",
            "type": "rect",
            "x": %d,
            "y": %d,
            "width": %d,
            "height": %d
        }
    ]], 
    string.format("%08x-%04x-%04x-%04x-%012x", 
        math.random(0, 0xffffffff),
        math.random(0, 0xffff),
        math.random(0, 0xffff),
        math.random(0, 0xffff),
        math.random(0, 0xffffffffffff)),
    id,
    math.random(-1000, 1000),
    math.random(-1000, 1000),
    math.random(10, 500),
    math.random(10, 500))
end

local function generate_request_body(num_shapes)
    local shapes = {}
    for i = 1, num_shapes do
        table.insert(shapes, generate_shape(i))
    end
    return '{"shapes": [' .. table.concat(shapes, ",") .. ']}'
end

-- Number of shapes per request (adjust as needed)
local NUM_SHAPES = 10

-- Pre-generate request bodies for variety
local bodies = {}
for i = 1, 100 do
    bodies[i] = generate_request_body(NUM_SHAPES)
end

local counter = 0

function request()
    counter = counter + 1
    local body = bodies[(counter % 100) + 1]
    
    return wrk.format("POST", "/validate", {
        ["Content-Type"] = "application/json",
        ["Accept"] = "application/json"
    }, body)
end

function response(status, headers, body)
    if status ~= 200 and status ~= 400 then
        print("Unexpected status: " .. status)
    end
end

function done(summary, latency, requests)
    io.write("\n")
    io.write("=== Shape Validator Benchmark Results ===\n")
    io.write(string.format("  Shapes per request: %d\n", NUM_SHAPES))
    io.write(string.format("  Requests/sec: %.2f\n", summary.requests / (summary.duration / 1000000)))
    io.write(string.format("  Shapes/sec: %.2f\n", (summary.requests * NUM_SHAPES) / (summary.duration / 1000000)))
    io.write(string.format("  Avg latency: %.2f ms\n", latency.mean / 1000))
    io.write(string.format("  P99 latency: %.2f ms\n", latency:percentile(99) / 1000))
    io.write(string.format("  Total requests: %d\n", summary.requests))
    io.write(string.format("  Total errors: %d\n", summary.errors.status + summary.errors.connect + summary.errors.read + summary.errors.write + summary.errors.timeout))
    io.write("==========================================\n")
end
