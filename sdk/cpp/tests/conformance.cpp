#include "mas.hpp"
#include <cassert>
#include <limits>
#include <filesystem>
#include <fstream>
#include <thread>

int main() {
    using mas::Json;
    const auto text = R"({"n":18446744073709551615,"s":"∀🧠","zero":0})";
    assert(mas::detail::parse(text).at("n").get<std::uint64_t>() == UINT64_MAX);
    for (const auto& invalid : {R"({"x":1,"x":2})", R"({"x":{"n":1,"\u006e":2}})",
            R"({"n":1.0})", R"({"n":1e2})", R"({"n":18446744073709551616})", R"({"s":"\ud800"})", "[]"}) {
        bool rejected=false;
        try { mas::detail::parse(invalid); } catch (const std::exception&) { rejected=true; }
        assert(rejected);
    }
    std::string nested="0";
    for (int i=0; i<33; ++i) nested="{\"x\":"+nested+"}";
    bool rejected=false;
    try { mas::detail::parse(nested); } catch (const std::exception&) { rejected=true; }
    assert(rejected);
    for (const auto number : {1.5, std::numeric_limits<double>::infinity(), std::numeric_limits<double>::quiet_NaN()}) {
        std::size_t budget=mas::max_frame;
        bool failed=false;
        try { mas::detail::preflight(Json{{"n",number}},0,budget); } catch (const std::exception&) { failed=true; }
        assert(failed);
    }
    assert(mas::detail::parse(R"({"a":{},"b":{},"c":[{"x":1},{"x":2}]})")["c"].size()==2);
    int sockets[2]; assert(::socketpair(AF_UNIX,SOCK_STREAM,0,sockets)==0);
    std::thread host([&] {
        assert(mas::detail::read_frame(sockets[0])["type"]=="effect_begin");
        mas::detail::write_frame(sockets[0],{{"type","effect"},{"state","cached"},{"output",{{"ok",true}}}});
        assert(mas::detail::read_frame(sockets[0])["requests"].size()==2);
        mas::detail::write_frame(sockets[0],{{"type","response"},{"ok",true},{"outputs",Json::array({{{"n",1}},{{"n",2}}})}});
        assert(mas::detail::read_frame(sockets[0])["type"]=="effect_begin");
        mas::detail::write_frame(sockets[0],{{"type","effect"},{"state","execute"},{"invocation_id","effect-1"}});
        assert(mas::detail::read_frame(sockets[0])["evidence"]["receipt"]=="effect-1");
        mas::detail::write_frame(sockets[0],{{"type","response"},{"ok",true}});
        ::close(sockets[0]);
    });
    mas::Context context(sockets[1],{{"actor","invoice"},{"invocation_id","one"},{"deadline_ms",0}});
    bool executed=false;
    auto cached=context.effect("one",Json::object(),[&](const std::string&) { executed=true; return std::make_pair(Json::object(),Json::object()); });
    assert(cached["ok"]==true && !executed);
    auto outputs=context.parallel({{"first","a",Json::object()},{"second","b",Json::object()}});
    assert(outputs[0]["n"]==1 && outputs[1]["n"]==2);
    context.effect("two",Json::object(),[](const std::string& id) { return std::make_pair(Json{{"done",true}},Json{{"receipt",id}}); });
    ::close(sockets[1]); host.join();
    std::ifstream fixture_file(std::filesystem::path(__FILE__).parent_path().parent_path().parent_path()/"completion-conformance.json");
    assert(fixture_file.good());
    const auto fixture=Json::parse(fixture_file);
    for (const auto& test : fixture.at("cases")) {
        int pair[2]; assert(::socketpair(AF_UNIX,SOCK_STREAM,0,pair)==0);
        const bool valid=test.at("valid").get<bool>();
        const Json expected=valid ? test.at("output") : Json{{"redacted",true}};
        std::thread projected_host([&] {
            assert(mas::detail::read_frame(pair[0])["type"]=="effect_begin");
            mas::detail::write_frame(pair[0],{{"type","effect"},{"state","execute"},{"invocation_id","effect-1"}});
            assert(mas::detail::read_frame(pair[0])["output"]==Json({{"private","private-local-output"}}));
            mas::detail::write_frame(pair[0],{{"type","response"},{"ok",true},{"completion",test.at("completion")}});
            assert(mas::detail::read_frame(pair[0])["type"]=="effect_begin");
            mas::detail::write_frame(pair[0],{{"type","effect"},{"state","cached"},{"output",expected}});
            ::close(pair[0]);
        });
        mas::Context projected_context(pair[1],{{"actor","invoice"},{"invocation_id","one"},{"deadline_ms",0}});
        int calls=0;
        const auto callback=[&](const std::string& id) {
            assert(id=="effect-1"); ++calls;
            return std::make_pair(Json{{"private","private-local-output"}},Json{{"receipt","one"}});
        };
        bool failed=false;
        try { const auto fresh=projected_context.effect("write",Json::object(),callback); assert(valid && fresh==expected); }
        catch (const std::runtime_error& error) { failed=true; assert(!valid && std::string(error.what())=="invalid governed effect completion"); }
        assert(failed!=valid);
        assert(projected_context.effect("write",Json::object(),callback)==expected && calls==1);
        ::close(pair[1]); projected_host.join();
    }
}
