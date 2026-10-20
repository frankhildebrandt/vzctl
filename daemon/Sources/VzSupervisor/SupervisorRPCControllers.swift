import Foundation
import VzDaemonKit

extension SupervisorServer {
    func handleControllerRPC(_ request: JSONRPCRequest) -> JSONRPCResponse? {
        guard ["controller.register", "controller.unregister", "controller.registration"].contains(request.method) else { return nil }
        do {
            let params = try objectParams(request.params, context: request.method)
            let project = try requiredReconcileString("project", from: params)
            switch request.method {
            case "controller.register":
                let configPath = try requiredReconcileString("config_path", from: params)
                _ = try controllerReconciler.register(project: project, configPath: configPath)
            case "controller.unregister": try controllerReconciler.unregister(project: project)
            default: break
            }
            return JSONRPCResponse(result: controllerReconciler.registration(project: project), id: request.id ?? .null)
        } catch { return reconcileErrorResponse(error, request: request) }
    }
}
