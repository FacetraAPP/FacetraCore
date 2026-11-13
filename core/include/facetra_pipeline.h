#ifndef FACETRA_PIPELINE_H
#define FACETRA_PIPELINE_H
#ifdef __cplusplus
extern "C" {
#endif
/* UTF-8 JSON request, synchronous: run on a worker thread.
 * Request: documents (raw YAML strings), chart_id, directory, params (optional),
 * include_frame (optional, defaults true for parity diagnostics).
 * With include_frame=false, ok contains the final renderer payload, engine
 * version and timings but no duplicate frame. Source tables stay in Rust.
 * Returns UTF-8 JSON {"ok":...} or {"error":...}; caller must free once.
 * Successful envelopes also report boundary_ms request_decode/response_encode.
 * Input must be a valid NUL-terminated string (or NULL).
 * Output must only be released through facetra_pipeline_free, never free(). */
char *facetra_pipeline_run(const char *input);
void facetra_pipeline_free(char *output);
/* Pure YAML Contract API: no session, filesystem access or execution engine.
 * JSON {op:info|schema|decode|validate|parameters|definitions|source|target,...}.
 * Returns {ok:...} or {error:{code,path,resource?}}. Free with facetra_pipeline_free. */
char *facetra_contract_run(const char *input);
/* Application ABI v1. JSON uses {version:1, op=resolve|query|render_frame|validate_render|compile_sql|...}.
 * Session calls are serialized internally. A session retains at most one
 * immutable DuckDB source snapshot (128 MiB native memory limit).
 * A cancellation belongs to one call and may be requested concurrently.
 * Keep all handles alive until run/cancel return. Destroy each handle once;
 * destroy must never race any other call. Never reuse a cancellation handle.
 * All returned strings use facetra_pipeline_free. */
typedef struct FacetraSession FacetraSession;
typedef struct FacetraCancellation FacetraCancellation;
/* libgit2 JSON operations. Run off the UI thread. UTF-8 response is owned by
 * the caller and freed with facetra_pipeline_free. Credentials are used only
 * for transport callbacks and never written to repository configuration. */
char *facetra_git_run(const char *input, const FacetraCancellation *cancel);
FacetraSession *facetra_session_create(void);
void facetra_session_destroy(FacetraSession *session);
FacetraCancellation *facetra_cancel_create(void);
void facetra_cancel_request(const FacetraCancellation *cancel);
void facetra_cancel_destroy(FacetraCancellation *cancel);
char *facetra_session_run(const FacetraSession *session, const char *input,
                         const FacetraCancellation *cancel);
#ifdef __cplusplus
}
#endif
#endif
