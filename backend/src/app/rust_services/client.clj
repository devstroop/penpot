;; Rust Services Integration Layer
;; ================================
;; This namespace provides integration with high-performance Rust microservices.
;; These services can be enabled/disabled via feature flags.

(ns app.rust-services.client
  "HTTP client for Rust microservices"
  (:require
   [app.common.logging :as log]
   [app.config :as cfg]
   [clojure.core.async :as a]
   [promesa.core :as p]
   [promesa.exec :as px])
  (:import
   java.net.URI
   java.net.http.HttpClient
   java.net.http.HttpRequest
   java.net.http.HttpRequest$BodyPublishers
   java.net.http.HttpResponse$BodyHandlers
   java.time.Duration))

;; ---------------------------------------------------------------------------
;; Configuration
;; ---------------------------------------------------------------------------

(def ^:private default-timeout-ms 5000)

(defn- get-service-url
  "Get the URL for a Rust service from config"
  [service-key]
  (case service-key
    :shape-validator (cfg/get :penpot-shape-validator-url "http://localhost:8081")
    :realtime-sync   (cfg/get :penpot-realtime-url "http://localhost:8082")
    :render-service  (cfg/get :penpot-render-service-url "http://localhost:8083")
    (throw (ex-info "Unknown Rust service" {:service service-key}))))

(defn- rust-services-enabled?
  "Check if Rust services integration is enabled"
  []
  (cfg/get :penpot-rust-services-enabled false))

;; ---------------------------------------------------------------------------
;; HTTP Client
;; ---------------------------------------------------------------------------

(defonce ^:private http-client
  (delay
    (-> (HttpClient/newBuilder)
        (.connectTimeout (Duration/ofMillis 2000))
        (.build))))

(defn- make-request
  "Make an HTTP request to a Rust service"
  [{:keys [method url body timeout-ms]
    :or {method :get timeout-ms default-timeout-ms}}]
  (let [builder (-> (HttpRequest/newBuilder)
                    (.uri (URI/create url))
                    (.timeout (Duration/ofMillis timeout-ms))
                    (.header "Content-Type" "application/json")
                    (.header "Accept" "application/json"))]
    (case method
      :get  (.GET builder)
      :post (.POST builder (HttpRequest$BodyPublishers/ofString (or body "{}")))
      :put  (.PUT builder (HttpRequest$BodyPublishers/ofString (or body "{}"))))
    (.build builder)))

(defn- send-request
  "Send HTTP request and return response"
  [request]
  (p/create
   (fn [resolve reject]
     (px/run!
      (fn []
        (try
          (let [response (.send @http-client request (HttpResponse$BodyHandlers/ofString))
                status   (.statusCode response)
                body     (.body response)]
            (if (< status 400)
              (resolve {:status status :body body})
              (reject (ex-info "Rust service error" {:status status :body body}))))
          (catch Exception e
            (reject e))))))))

;; ---------------------------------------------------------------------------
;; Service Health Checks
;; ---------------------------------------------------------------------------

(defn check-service-health
  "Check if a Rust service is healthy"
  [service-key]
  (p/let [url     (str (get-service-url service-key) "/health")
          request (make-request {:method :get :url url :timeout-ms 2000})
          result  (p/catch (send-request request) (constantly nil))]
    (boolean result)))

(defn check-all-services
  "Check health of all Rust services"
  []
  (p/let [validator (check-service-health :shape-validator)
          realtime  (check-service-health :realtime-sync)
          render    (check-service-health :render-service)]
    {:shape-validator validator
     :realtime-sync   realtime
     :render-service  render}))

;; ---------------------------------------------------------------------------
;; Shape Validator Integration
;; ---------------------------------------------------------------------------

(defn validate-shapes-rust
  "Validate shapes using the Rust validator service.
   Returns a promise with validation result."
  [shapes]
  (if-not (rust-services-enabled?)
    (p/resolved {:valid true :source :disabled})
    (p/let [url     (str (get-service-url :shape-validator) "/validate")
            body    (app.common.json/encode {:shapes shapes})
            request (make-request {:method :post :url url :body body})
            result  (send-request request)]
      (-> result
          :body
          app.common.json/decode
          (assoc :source :rust)))))

(defn validate-shapes-with-fallback
  "Validate shapes using Rust service, falling back to Clojure on failure.
   The `clojure-validator-fn` should be a function that takes shapes and validates them."
  [shapes clojure-validator-fn]
  (if-not (rust-services-enabled?)
    (clojure-validator-fn shapes)
    (-> (validate-shapes-rust shapes)
        (p/catch
         (fn [error]
           (log/warn :msg "Rust validator failed, falling back to Clojure"
                     :error (ex-message error))
           (clojure-validator-fn shapes))))))

;; ---------------------------------------------------------------------------
;; Render Service Integration
;; ---------------------------------------------------------------------------

(defn render-page-rust
  "Request server-side rendering from Rust service"
  [{:keys [file-id page-id format scale shapes]}]
  (if-not (rust-services-enabled?)
    (p/resolved {:success false :reason :disabled})
    (p/let [url     (str (get-service-url :render-service) "/render")
            body    (app.common.json/encode
                     {:file_id  file-id
                      :page_id  page-id
                      :format   (name format)
                      :scale    (or scale 1.0)
                      :shapes   shapes})
            request (make-request {:method :post :url url :body body})
            result  (send-request request)]
      (-> result
          :body
          app.common.json/decode))))

(defn generate-thumbnail-rust
  "Generate thumbnail using Rust service"
  [{:keys [file-id page-id]}]
  (if-not (rust-services-enabled?)
    (p/resolved {:success false :reason :disabled})
    (p/let [url     (str (get-service-url :render-service) "/thumbnail")
            body    (app.common.json/encode
                     {:file_id file-id
                      :page_id page-id
                      :format  "png"})
            request (make-request {:method :post :url url :body body})
            result  (send-request request)]
      (-> result
          :body
          app.common.json/decode))))

;; ---------------------------------------------------------------------------
;; WebSocket / Real-time Sync
;; ---------------------------------------------------------------------------

(defn get-realtime-ws-url
  "Get the WebSocket URL for real-time sync"
  [file-id]
  (let [base-url (get-service-url :realtime-sync)]
    (str (clojure.string/replace base-url #"^http" "ws") "/ws/" file-id)))

;; ---------------------------------------------------------------------------
;; Initialization
;; ---------------------------------------------------------------------------

(defn init!
  "Initialize Rust services integration.
   Checks health of all services and logs status."
  []
  (when (rust-services-enabled?)
    (log/info :msg "Rust services integration enabled, checking health...")
    (p/let [health (check-all-services)]
      (doseq [[service healthy?] health]
        (if healthy?
          (log/info :msg "Rust service healthy" :service service)
          (log/warn :msg "Rust service not available" :service service)))
      health)))
