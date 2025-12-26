;; Rust Shape Validator Integration
;; =================================
;; Drop-in replacement for Malli-based shape validation using Rust.

(ns app.rust-services.shapes
  "Shape validation using Rust microservice"
  (:require
   [app.common.logging :as log]
   [app.rust-services.client :as rust]
   [promesa.core :as p]))

(defn validate-shape
  "Validate a single shape using Rust service.
   Returns a promise with validation result."
  [shape]
  (rust/validate-shapes-rust [shape]))

(defn validate-shapes
  "Validate multiple shapes using Rust service.
   Returns a promise with validation result."
  [shapes]
  (rust/validate-shapes-rust shapes))

(defn valid?
  "Check if shapes are valid. Returns a promise resolving to boolean."
  [shapes]
  (p/let [result (validate-shapes shapes)]
    (:valid result)))

(defn validation-errors
  "Get validation errors for shapes. Returns a promise."
  [shapes]
  (p/let [result (validate-shapes shapes)]
    (when-not (:valid result)
      (->> (:results result)
           (filter #(not (:valid %)))
           (mapcat :errors)))))

;; ---------------------------------------------------------------------------
;; Hybrid Validation (Rust + Clojure fallback)
;; ---------------------------------------------------------------------------

(defn make-hybrid-validator
  "Create a hybrid validator that uses Rust when available,
   falling back to the provided Clojure validator function."
  [clojure-validate-fn]
  (fn [shapes]
    (rust/validate-shapes-with-fallback shapes clojure-validate-fn)))

;; ---------------------------------------------------------------------------
;; Performance Monitoring
;; ---------------------------------------------------------------------------

(defn benchmark-validation
  "Benchmark validation performance.
   Runs validation multiple times and returns timing statistics."
  [shapes iterations]
  (p/let [start-time (System/nanoTime)
          _          (p/loop [i 0]
                       (when (< i iterations)
                         (p/let [_ (validate-shapes shapes)]
                           (p/recur (inc i)))))
          end-time   (System/nanoTime)
          total-ms   (/ (- end-time start-time) 1000000.0)
          per-call   (/ total-ms iterations)]
    {:total-ms    total-ms
     :iterations  iterations
     :per-call-ms per-call
     :shapes      (count shapes)
     :per-shape-us (/ (* per-call 1000) (count shapes))}))
