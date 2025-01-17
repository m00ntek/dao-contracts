import { Addr, Duration } from "./shared-types";

export interface InstantiateMsg {
admin: Addr
token_address: Addr
unstaking_duration?: (Duration | null)
[k: string]: unknown
}
