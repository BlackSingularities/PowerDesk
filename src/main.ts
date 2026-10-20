import { mount } from 'svelte'
import './app.css'
import App from './App.svelte'
import ViewerApp from './ViewerApp.svelte'

// To samo wejście obsługuje główne okno i osobne okno podglądu (#viewer).
const Root = location.hash === '#viewer' ? ViewerApp : App
export default mount(Root, { target: document.getElementById('app')! })
