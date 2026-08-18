# Changelog

## [0.3.0](https://github.com/pactflow/pact-mcp-plugin/compare/v0.2.0...v0.3.0) (2026-08-18)


### Features

* **auth:** enable rmcp OAuth2 auth feature + fix rustls crypto-provider panic ([6e233ae](https://github.com/pactflow/pact-mcp-plugin/commit/6e233aec91a147a1e2ab5c24ed8d944be16cc6d3))
* **auth:** OAuth2 client-credentials for MCP provider verification ([8cc00dd](https://github.com/pactflow/pact-mcp-plugin/commit/8cc00dd98cf832b94e9ebbb7ffd26f4a76c35656))
* **auth:** OAuth2 client-credentials transport path (ADR 0011) ([50a6bad](https://github.com/pactflow/pact-mcp-plugin/commit/50a6bad8f422d9ddc6f792c50897a87dce9295c9))
* **auth:** parse + carry OAuth2 client-credentials config (ADR 0011) ([f7d67e6](https://github.com/pactflow/pact-mcp-plugin/commit/f7d67e65328f12641c39eec210c9a6dd3d97f50a))
* **ts:** add oauth arm to HttpAuth (ADR 0011) ([bd411c9](https://github.com/pactflow/pact-mcp-plugin/commit/bd411c92499477befa04265db6fcf19044a2d2cc))

## [0.2.0](https://github.com/mefellows/pact-mcp-plugin/compare/v0.1.0...v0.2.0) (2026-07-31)


### Features

* **engine:** quiet logging by default; honor PACT_MCP_LOG / RUST_LOG ([df3556a](https://github.com/mefellows/pact-mcp-plugin/commit/df3556a0a9c74686c21e3c4d816a429a9b1a5c9f))


### Bug Fixes

* **release:** build both macOS arches on the arm runner; add dispatch recovery ([d910ee3](https://github.com/mefellows/pact-mcp-plugin/commit/d910ee3c79c3fa9c8c9e9d0084ecbb9f7c5fc88a))

## 0.1.0 (2026-07-30)


### Features

* **engine:** compare + verify CLI subcommands for the TS adapter ([7712950](https://github.com/mefellows/pact-mcp-plugin/commit/771295060820c495336ce00164030080c23c95fa))
* **engine:** gRPC bootstrap + PactPlugin dispatch (InitPlugin..VerifyInteraction) ([f9fda68](https://github.com/mefellows/pact-mcp-plugin/commit/f9fda686081d1c0210c39e6acebf03894d103e75))
* **engine:** HTTP provider verification + auth-protected fixture server ([a910f9a](https://github.com/mefellows/pact-mcp-plugin/commit/a910f9a91ff0039dc7bda8d7a091c412ace3a701))
* **engine:** loopback HTTP consumer mock + mcp-http catalogue entry ([893f346](https://github.com/mefellows/pact-mcp-plugin/commit/893f3463cb7937b782b4192c5e11122728f3261d))
* **engine:** mcp model serde types + JSON-RPC envelope synthesis ([26beb76](https://github.com/mefellows/pact-mcp-plugin/commit/26beb764cd9650d889bf91337412fd67755e3bf0))
* **engine:** MCP-aware content matching + conformance test harness ([532b854](https://github.com/mefellows/pact-mcp-plugin/commit/532b854035ead94d84b0447547b686bb9f4456f8))
* **engine:** Phase 1.8 stdio mock mode + StartMockServer handoff ([95db49c](https://github.com/mefellows/pact-mcp-plugin/commit/95db49ce2c60c01f15033923f46331f66ecd35de))
* **engine:** scaffold Cargo workspace, vendor plugin.proto, wire tonic-build ([985a2cc](https://github.com/mefellows/pact-mcp-plugin/commit/985a2cc4836f4b29ec6d4dfb1f803e4755fcf810))
* **engine:** standard-verifier VerifyInteraction — transport routing + config ladder (ADR 0008) ([5b6a7cb](https://github.com/mefellows/pact-mcp-plugin/commit/5b6a7cb760c73f95e085cb558233f5106361bb41))
* **engine:** stdio transport (rmcp) + handshake + provider verification ([e31f7d5](https://github.com/mefellows/pact-mcp-plugin/commit/e31f7d5202fc139ecb9ed294bd281b299291771a))
* **engine:** Streamable HTTP client transport + AuthProvider ([1a90f24](https://github.com/mefellows/pact-mcp-plugin/commit/1a90f24725263c3cf71a51e1eea7539072136329))
* **examples:** provider-stdio — Phase 1 demo, verified end-to-end ([ad2b824](https://github.com/mefellows/pact-mcp-plugin/commit/ad2b824fdae50e26fe26413ab1a582c0b0b7abf9))
* **fixtures:** add real minimal MCP stdio server (get_weather) for testing ([da7fd32](https://github.com/mefellows/pact-mcp-plugin/commit/da7fd322c5df70def22f2c9d1dd145cc9b5f89e8))
* **mock:** carry request-side matchers from the pact into the mock ([7f5f221](https://github.com/mefellows/pact-mcp-plugin/commit/7f5f221add7bd03ba65d054dadf889790854d6ee))
* **npm:** publish as @pactflow/pact-mcp-plugin with engine auto-provisioning ([ec68780](https://github.com/mefellows/pact-mcp-plugin/commit/ec687802a54594101bb3f849b7885f8ca121bab7))
* provider states — given() DSL, engine env seeding, stateHandlers (ADR 0009) ([4fde51c](https://github.com/mefellows/pact-mcp-plugin/commit/4fde51ceadd6adec71d3089ccba7eb8cf7922553))
* resources/read|list + prompts/get|list vertical slice (G5) ([5b88aab](https://github.com/mefellows/pact-mcp-plugin/commit/5b88aaba1a24ebae9aa7999182c50257cd8d8934))
* standard pact-js Verifier E2E green over mcp-http + mcp-stdio (ADR 0008) ([0fedf0e](https://github.com/mefellows/pact-mcp-plugin/commit/0fedf0e8219f28b4b6b8bcd1bfc289edc3a1782e))
* **ts:** HTTP provider verification (with auth) + HTTP consumer mock ([edb8d8a](https://github.com/mefellows/pact-mcp-plugin/commit/edb8d8ac2114de791a049d498dfe91a6e299b704))
* **ts:** multi-interaction DSL + expectsToolsList (G4) ([89258e0](https://github.com/mefellows/pact-mcp-plugin/commit/89258e0406855e3bce763c83048116aa47578c2d))
* **ts:** stamp transport + key onto emitted pacts (ADR 0008) ([db5df8d](https://github.com/mefellows/pact-mcp-plugin/commit/db5df8de07678920442563843587bef59347b81a))
* **ts:** TypeScript adapter (McpPact + McpProviderVerifier) with example tests ([09ba3f5](https://github.com/mefellows/pact-mcp-plugin/commit/09ba3f53bd0a3a4383102e04e8ef958bebe71bf2))


### Bug Fixes

* **engine:** align ConfigureInteraction with real pact inline-DSL convention ([2fdce7e](https://github.com/mefellows/pact-mcp-plugin/commit/2fdce7e0db9d892900661d9ff0d68819f3a38931))
* **engine:** escape + in content-type regex — unblocks live pact-js round trip ([9cdcdfb](https://github.com/mefellows/pact-mcp-plugin/commit/9cdcdfb8864cd70ed1f06a96841787e2b3cbf53f))
* **engine:** two-part sync-message ConfigureInteraction; live pact-js probe ([03b9a04](https://github.com/mefellows/pact-mcp-plugin/commit/03b9a042d2a1e0dc34b92439f2c4b734e0937956))


### Miscellaneous Chores

* release 0.1.0 ([7149c43](https://github.com/mefellows/pact-mcp-plugin/commit/7149c43894ed27115d11859bad00cabf1e22a07c))
