;; Rust Services Integration Layer
;; ================================
;; This namespace provides integration with high-performance Rust microservices.
;; These services can be enabled/disabled via feature flags.
;;
;; Environment Variables:
;;   PENPOT_RUST_SERVICES_ENABLED - Enable Rust services (default: false)
;;   PENPOT_SHAPE_VALIDATOR_URL   - Shape validator URL (default: http://localhost:8081)
;;   PENPOT_RENDER_SERVICE_URL    - Render service URL (default: http://localhost:8083)
;;   PENPOT_REALTIME_URL          - Realtime sync URL (default: http://localhost:8082)
;;   PENPOT_API_GATEWAY_URL       - API gateway URL (default: http://localhost:8080)

(ns app.rust-services.client
  "HTTP client for Rust microservices"
  (:require
   [app.common.logging :as log]
   [app.config :as cfg]
   [app.metrics :as mtx]
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
(def ^:private default-connect-timeout-ms 2000)

(defn- get-service-url
  "Get the URL for a Rust service from config"
  [service-key]
  (case service-key
    :shape-validator (cfg/get :penpot-shape-validator-url "http://localhost:8081")
    :realtime-sync   (cfg/get :penpot-realtime-url "http://localhost:8082")
    :render-service  (cfg/get :penpot-render-service-url "http://localhost:8083")
    :api-gateway     (cfg/get :penpot-api-gateway-url "http://localhost:8080")
    (throw (ex-info "Unknown Rust service" {:service service-key}))))

(defn rust-services-enabled?
  "Check if Rust services integration is enabled"
  []
  (cfg/get :penpot-rust-services-enabled false))

;; ---------------------------------------------------------------------------
;; Metrics
;; ---------------------------------------------------------------------------

(defonce rust-request-duration-histogram
  (mtx/create-histogram
   {:name "penpot_rust_service_request_duration_seconds"
    :help "Duration of requests to Rust services"
    :labels ["service" "endpoint" "status"]}))

(defonce rust-request-counter
  (mtx/create-counter
   {:name "penpot_rust_service_requests_total"
    :help "Total requests to Rust services"
    :labels ["service" "endpoint" "status"]}))

(defn- record-request-metrics
  "Record metrics for a Rust service request"
  [service endpoint status duration-ms]
  (let [service-name (name service)
        status-str   (str status)]
    (mtx/observe! rust-request-duration-histogram 
                  (/ duration-ms 1000.0) 
                  service-name endpoint status-str)
    (mtx/inc! rust-request-counter service-name endpoint status-str)))

;; ---------------------------------------------------------------------------
;; HTTP Client
;; ---------------------------------------------------------------------------

(defonce ^:private http-client
  (delay
    (-> (HttpClient/newBuilder)
        (.connectTimeout (Duration/ofMillis default-connect-timeout-ms))
        (.followRedirects java.net.http.HttpClient$Redirect/NORMAL)
        (.build))))

(defn- make-request
  "Make an HTTP request to a Rust service"
  [{:keys [method url body timeout-ms headers]
    :or {method :get timeout-ms default-timeout-ms headers {}}}]
  (let [builder (-> (HttpRequest/newBuilder)
                    (.uri (URI/create url))
                    (.timeout (Duration/ofMillis timeout-ms))
                    (.header "Content-Type" "application/json")
                    (.header "Accept" "application/json")
                    (.header "X-Request-Source" "penpot-clojure"))]
    ;; Add custom headers
    (doseq [[k v] headers]
      (.header builder (name k) (str v)))
    
    (case method
      :get    (.GET builder)
      :post   (.POST builder (HttpRequest$BodyPublishers/ofString (or body "{}")))
      :put    (.PUT builder (HttpRequest$BodyPublishers/ofString (or body "{}")))
      :delete (.DELETE builder))
    (.build builder)))

(defn- send-request
  "Send HTTP request and return response"
  [request service endpoint]
  (let [start-time (System/currentTimeMillis)]
    (p/create
     (fn [resolve reject]
       (px/run!
        (fn []
          (try
            (let [response     (.send @http-client request (HttpResponse$BodyHandlers/ofString))
                  status       (.statusCode response)
                  body         (.body response)
                  duration-ms  (- (System/currentTimeMillis) start-time)]
              (record-request-metrics service endpoint status duration-ms)
              (if (< status 400)
                (resolve {:status status :body body :duration-ms duration-ms})
                (reject (ex-info "Rust service error" 
                                 {:status status :body body :service service}))))
            (catch java.net.ConnectException e
              (let [duration-ms (- (System/currentTimeMillis) start-time)]
                (record-request-metrics service endpoint 0 duration-ms)
                (reject (ex-info "Rust service connection failed" 
                                 {:service service :error (ex-message e)}))))
            (catch java.net.http.HttpTimeoutException e
              (let [duration-ms (- (System/currentTimeMillis) start-time)]
                (record-request-metrics service endpoint 408 duration-ms)
                (reject (ex-info "Rust service timeout" 
                                 {:service service :timeout-ms default-timeout-ms}))))
            (catch Exception e
              (let [duration-ms (- (System/currentTimeMillis) start-time)]
                (record-request-metrics service endpoint 500 duration-ms)
                (reject e))))))))))

(defn call-service
  "Make a call to a Rust service with automatic JSON encoding/decoding"
  [{:keys [service endpoint method body timeout-ms]
    :or {method :get timeout-ms default-timeout-ms}}]
  (let [base-url (get-service-url service)
        url      (str base-url endpoint)
        json-body (when body (app.common.json/encode body))
        request  (make-request {:method method :url url :body json-body :timeout-ms timeout-ms})]
    (p/let [result (send-request request service endpoint)]
      (-> result
          (update :body app.common.json/decode)
          (assoc :source :rust)))))

;; ---------------------------------------------------------------------------
;; Service Health Checks
;; ---------------------------------------------------------------------------

(defn check-service-health
  "Check if a Rust service is healthy"
  [service-key]
  (p/let [url     (str (get-service-url service-key) "/health")
          request (make-request {:method :get :url url :timeout-ms 2000})
          result  (p/catch 
                   (send-request request service-key "/health") 
                   (constantly nil))]
    (if result
      {:healthy true :service service-key :response (:body result)}
      {:healthy false :service service-key})))

(defn check-all-services
  "Check health of all Rust services"
  []
  (p/let [validator (check-service-health :shape-validator)
          realtime  (check-service-health :realtime-sync)
          render    (check-service-health :render-service)
          gateway   (check-service-health :api-gateway)]
    {:shape-validator (:healthy validator)
     :realtime-sync   (:healthy realtime)
     :render-service  (:healthy render)
     :api-gateway     (:healthy gateway)
     :all-healthy     (and (:healthy validator)
                           (:healthy realtime)
                           (:healthy render)
                           (:healthy gateway))}))

;; ---------------------------------------------------------------------------
;; Shape Validator Integration
;; ---------------------------------------------------------------------------

(defn validate-shapes-rust
  "Validate shapes using the Rust validator service.
   Returns a promise with validation result."
  [shapes]
  (if-not (rust-services-enabled?)
    (p/resolved {:valid true :source :disabled})
    (call-service
     {:service  :shape-validator
      :endpoint "/validate"
      :method   :post
      :body     {:shapes shapes}})))

(defn validate-shapes-with-fallback
  "Validate shapes using Rust service, falling back to Clojure on failure.
   The `clojure-validator-fn` should be a function that takes shapes and validates them."
  [shapes clojure-validator-fn]
  (if-not (rust-services-enabled?)
    (clojure-validator-fn shapes)
    (-> (validate-shapes-rust shapes)
        (p/then (fn [result] (assoc result :source :rust)))
        (p/catch
         (fn [error]
           (log/warn :msg "Rust validator failed, falling back to Clojure"
                     :error (ex-message error))
           (let [result (clojure-validator-fn shapes)]
             (assoc result :source :clojure-fallback)))))))

;; ---------------------------------------------------------------------------
;; Render Service Integration
;; ---------------------------------------------------------------------------

(defn render-page-rust
  "Request server-side rendering from Rust service"
  [{:keys [file-id page-id format scale shapes]}]
  (if-not (rust-services-enabled?)
    (p/resolved {:success false :reason :disabled})
    (call-service
     {:service  :render-service
      :endpoint "/render"
      :method   :post
      :body     {:file_id  file-id
                 :page_id  page-id
                 :format   (name (or format :png))
                 :scale    (or scale 1.0)
                 :shapes   shapes}})))

(defn generate-thumbnail-rust
  "Generate thumbnail using Rust service"
  [{:keys [file-id page-id width height]}]
  (if-not (rust-services-enabled?)
    (p/resolved {:success false :reason :disabled})
    (call-service
     {:service  :render-service
      :endpoint "/thumbnail"
      :method   :post
      :body     {:file_id file-id
                 :page_id page-id
                 :width   (or width 300)
                 :height  (or height 150)
                 :format  "png"}})))

(defn render-svg-to-png
  "Render raw SVG to PNG using Rust service"
  [svg-content {:keys [width height scale]}]
  (if-not (rust-services-enabled?)
    (p/resolved {:success false :reason :disabled})
    (call-service
     {:service  :render-service
      :endpoint "/render-svg"
      :method   :post
      :body     {:svg    svg-content
                 :width  (or width 800)
                 :height (or height 600)
                 :scale  (or scale 1.0)}})))

;; ---------------------------------------------------------------------------
;; WebSocket / Real-time Sync
;; ---------------------------------------------------------------------------

(defn get-realtime-ws-url
  "Get the WebSocket URL for real-time sync"
  [file-id]
  (let [base-url (get-service-url :realtime-sync)]
    (str (clojure.string/replace base-url #"^http" "ws") "/ws/" file-id)))

(defn get-realtime-stats
  "Get realtime service statistics"
  []
  (if-not (rust-services-enabled?)
    (p/resolved {:available false :reason :disabled})
    (call-service
     {:service  :realtime-sync
      :endpoint "/stats"
      :method   :get})))

;; ---------------------------------------------------------------------------
;; API Gateway Integration
;; ---------------------------------------------------------------------------

(defn get-gateway-health
  "Get API gateway health including all service circuit breakers"
  []
  (call-service
   {:service  :api-gateway
    :endpoint "/health"
    :method   :get}))

(defn get-circuit-breakers
  "Get circuit breaker status from API gateway"
  []
  (call-service
   {:service  :api-gateway
    :endpoint "/circuits"
    :method   :get}))

;; ---------------------------------------------------------------------------
;; Initialization
;; ---------------------------------------------------------------------------

(defn init!
  "Initialize Rust services integration.
   Checks health of all services and logs status."
  []
  (if-not (rust-services-enabled?)
    (log/info :msg "Rust services integration DISABLED")
    (do
      (log/info :msg "Rust services integration ENABLED, checking health...")
      (p/let [health (check-all-services)]
        (log/info :msg "Rust services health check complete"
                  :all-healthy (:all-healthy health)
                  :details (dissoc health :all-healthy))
        (when-not (:all-healthy health)
          (log/warn :msg "Some Rust services are not available"
                    :status health))
        health))))
