#pragma once
// MAS-owned native framing and effect API. Linux, C++17, no application launcher.
#include "nlohmann/json.hpp"
#include <array>
#include <cerrno>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <functional>
#include <mutex>
#include <set>
#include <stdexcept>
#include <string>
#include <sys/socket.h>
#include <unistd.h>
#include <vector>

namespace mas {
using Json = nlohmann::json;
constexpr std::size_t max_frame = 65536;

namespace detail {
inline void object(const Json& value) {
    if (!value.is_object()) throw std::runtime_error("native input and output must be JSON objects");
}
inline Json parse(const std::string& text) {
    if (text.empty() || text.size() > max_frame) throw std::runtime_error("native frame exceeds limit");
    std::array<std::set<std::string>, 33> keys;
    auto callback = [&keys](int depth, Json::parse_event_t event, Json& value) {
        if (depth < 0 || depth > 32) throw std::runtime_error("native JSON exceeds nesting depth");
        if (event == Json::parse_event_t::object_start || event == Json::parse_event_t::array_start) {
            if (depth >= 32) throw std::runtime_error("native JSON exceeds nesting depth");
            keys.at(static_cast<std::size_t>(depth)).clear();
        }
        if (event == Json::parse_event_t::key) {
            if (depth == 0 || !keys.at(static_cast<std::size_t>(depth - 1)).insert(value.get<std::string>()).second)
                throw std::runtime_error("duplicate native JSON key");
        }
        if (event == Json::parse_event_t::value && value.is_number_float())
            throw std::runtime_error("C++ SDK accepts only signed or unsigned 64-bit integers; floats and larger integers are unsupported");
        return true;
    };
    auto value = Json::parse(text, callback);
    object(value);
    return value;
}
inline void exact_read(int fd, void* data, std::size_t length) {
    auto* p = static_cast<unsigned char*>(data);
    while (length) {
        const auto n = ::read(fd, p, length);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) throw std::runtime_error(n == 0 ? "native channel closed" : std::strerror(errno));
        p += n; length -= static_cast<std::size_t>(n);
    }
}
inline void exact_write(int fd, const void* data, std::size_t length) {
    const auto* p = static_cast<const unsigned char*>(data);
    while (length) {
        const auto n = ::send(fd, p, length, MSG_NOSIGNAL);
        if (n < 0 && errno == EINTR) continue;
        if (n <= 0) throw std::runtime_error("native channel write failed");
        p += n; length -= static_cast<std::size_t>(n);
    }
}
inline Json read_frame(int fd) {
    unsigned char header[4]; exact_read(fd, header, 4);
    const std::uint32_t length = (std::uint32_t(header[0]) << 24) | (std::uint32_t(header[1]) << 16)
        | (std::uint32_t(header[2]) << 8) | std::uint32_t(header[3]);
    if (length == 0 || length > max_frame) throw std::runtime_error("invalid native frame length");
    std::string text(length, '\0'); exact_read(fd, text.data(), length);
    return parse(text);
}
inline void preflight(const Json& value, std::size_t depth, std::size_t& budget) {
    if (value.is_number_float()) throw std::runtime_error("native JSON requires exact integers");
    if (value.is_binary() || value.is_discarded()) throw std::runtime_error("native value is not JSON");
    const auto charge = [&budget](std::size_t cost) {
        if (cost > budget) throw std::runtime_error("native frame exceeds limit");
        budget -= cost;
    };
    charge(1);
    if (value.is_string()) charge(value.get_ref<const std::string&>().size());
    if (value.is_structured()) {
        if (depth >= 32) throw std::runtime_error("native JSON exceeds nesting depth");
        for (auto it=value.begin(); it!=value.end(); ++it) {
            if (value.is_object()) charge(it.key().size());
            preflight(it.value(),depth+1,budget);
        }
    }
}
inline void write_frame(int fd, const Json& value) {
    object(value);
    std::size_t budget=max_frame; preflight(value,0,budget);
    const std::string text = value.dump();
    parse(text); // The parser validates the exact resulting bytes before writing.
    const auto n = static_cast<std::uint32_t>(text.size());
    const unsigned char header[] = {static_cast<unsigned char>(n >> 24), static_cast<unsigned char>(n >> 16),
        static_cast<unsigned char>(n >> 8), static_cast<unsigned char>(n)};
    exact_write(fd, header, 4); exact_write(fd, text.data(), text.size());
}
inline void success(const Json& value) {
    if (value.value("type", "") != "response" || !value.value("ok", false))
        throw std::runtime_error("invalid host response");
}
} // namespace detail

struct Request { std::string key; std::string target; Json input; };

// Not copyable: channel exchange is serialised so one command is outstanding.
class Context {
    int fd_;
    std::mutex channel_mutex_;
    Json exchange(const Json& command) {
        const std::lock_guard<std::mutex> lock(channel_mutex_);
        detail::write_frame(fd_, command);
        Json response = detail::read_frame(fd_);
        if (response.contains("ok") && response.at("ok") == false)
            throw std::runtime_error(response.value("error", "host rejected native command"));
        return response;
    }
    static Json request_value(const Request& request) {
        detail::object(request.input);
        if (request.key.empty() || request.target.empty()) throw std::runtime_error("request key and target must be non-empty");
        return {{"key",request.key},{"target",request.target},{"input",request.input}};
    }
public:
    const std::string actor;
    const std::string invocation_id;
    const std::uint64_t deadline_ms;
    Context(int fd, const Json& start) : fd_(fd), actor(start.at("actor").get<std::string>()),
        invocation_id(start.at("invocation_id").get<std::string>()), deadline_ms(start.at("deadline_ms").get<std::uint64_t>()) {}
    Context(const Context&) = delete;
    Context& operator=(const Context&) = delete;
    Json request(const std::string& key, const std::string& target, const Json& input) {
        auto command = request_value({key,target,input}); command["type"] = "request";
        auto response = exchange(command); detail::success(response);
        auto output = response.at("output"); detail::object(output); return output;
    }
    std::vector<Json> parallel(const std::vector<Request>& requests) {
        auto commands = Json::array();
        for (const auto& request : requests) commands.push_back(request_value(request));
        auto response = exchange({{"type","parallel"},{"requests",commands}}); detail::success(response);
        auto outputs = response.at("outputs").get<std::vector<Json>>();
        if (outputs.size() != requests.size()) throw std::runtime_error("host returned wrong number of parallel outputs");
        for (const auto& output : outputs) detail::object(output);
        return outputs;
    }
    // The callback receives the stable admitted effect identity and returns
    // {output,evidence}. Any exception after admission leaves uncertainty.
    template<class F> Json effect(const std::string& key, const Json& descriptor, F execute) {
        detail::object(descriptor);
        if (key.empty()) throw std::runtime_error("effect key must be non-empty");
        auto response = exchange({{"type","effect_begin"},{"key",key},{"descriptor",descriptor}});
        if (response.value("type", "") != "effect") throw std::runtime_error("invalid host effect response");
        const auto state = response.value("state", "");
        if (state == "cached") { auto output=response.at("output"); detail::object(output); return output; }
        if (state != "execute") throw std::runtime_error("invalid host effect state");
        const auto result = execute(response.at("invocation_id").get<std::string>());
        detail::object(result.first); detail::object(result.second);
        if (result.second.empty()) throw std::runtime_error("effect evidence must be non-empty");
        auto completed = exchange({{"type","effect_complete"},{"key",key},{"output",result.first},{"evidence",result.second}});
        detail::success(completed); return result.first;
    }
};

using Handler = Json (*)(Context&, const Json&);
// Called by generated worker launchers. The guardian discards stdout and stderr.
inline int serve(const std::string& handler_name, Handler handler) {
    int fd = -1;
    try {
        const char* raw = std::getenv("MAS_WORKER_CHANNEL_FD");
        if (!raw || !*raw) throw std::runtime_error("missing private worker channel");
        std::size_t used=0; fd=std::stoi(raw, &used);
        if (raw[used] != '\0' || fd < 3) throw std::runtime_error("invalid private worker channel");
        auto start = detail::read_frame(fd);
        if (start.value("type", "") != "start" || start.value("version", "") != "mas-native/1"
            || start.value("handler", "") != handler_name) throw std::runtime_error("invalid native start or handler");
        detail::object(start.at("input")); Context context(fd,start);
        const auto output = handler(context,start.at("input")); detail::object(output);
        detail::write_frame(fd, {{"type","complete"},{"output",output}});
        ::close(fd); return 0;
    } catch (const std::exception& error) {
        if (fd >= 3) {
            try { detail::write_frame(fd, {{"type","error"},{"message",error.what()}}); } catch (...) {}
            ::close(fd);
        }
        return 1;
    }
}
} // namespace mas
