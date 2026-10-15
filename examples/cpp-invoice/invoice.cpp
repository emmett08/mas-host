#include "mas.hpp"

mas::Json Assess(mas::Context&, const mas::Json& input) {
    // Currency amounts use integer minor units; negative and fractional values fail.
    for (const auto* field : {"amount_minor", "limit_minor"}) {
        const auto& value = input.at(field);
        if (!value.is_number_integer() || (value.is_number_integer() && !value.is_number_unsigned() && value.get<std::int64_t>() < 0))
            throw std::runtime_error(std::string(field) + " must be a non-negative 64-bit integer");
    }
    const auto amount = input.at("amount_minor").get<std::uint64_t>();
    const auto limit = input.at("limit_minor").get<std::uint64_t>();
    return {{"approved",amount <= limit},{"amount_minor",amount},{"limit_minor",limit}};
}
