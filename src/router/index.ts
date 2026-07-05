import { createRouter, createWebHistory } from "vue-router";

const router = createRouter({
  history: createWebHistory(),
  routes: [
    {
      path: "/",
      name: "dashboard",
      component: () => import("../pages/DashboardPage.vue"),
    },
    {
      path: "/config",
      name: "config",
      component: () => import("../pages/ConfigPage.vue"),
    },
    {
      path: "/settings",
      name: "settings",
      component: () => import("../pages/SettingsPage.vue"),
    },
    {
      path: "/logs",
      name: "logs",
      component: () => import("../pages/LogsPage.vue"),
    },
  ],
});

export default router;